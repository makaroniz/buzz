//! Fail-closed media classification, metadata removal, and output verification.
//!
//! Authentication is deliberately outside this module: the caller authenticates
//! the hash of the source bytes, while this module returns a new artifact whose
//! hash becomes the public content-addressed identifier.

use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::{Builder, NamedTempFile};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::{MediaConfig, MediaError};

const MAX_TOOL_OUTPUT: usize = 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 25_000_000;
const MAX_ANIMATION_FRAMES: u64 = 1_000;
const MAX_ANIMATION_PIXELS: u64 = 250_000_000;
const MAX_VIDEO_DURATION_SECS: f64 = 600.0;
const MAX_VIDEO_WIDTH: u32 = 3_840;
const MAX_VIDEO_HEIGHT: u32 = 2_160;

static TOOL_VERSIONS: OnceLock<ToolVersions> = OnceLock::new();

/// Sanitizer binary versions captured by the startup capability check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolVersions {
    /// Full first version line reported by ExifTool.
    pub exiftool: String,
    /// Full first version line reported by FFmpeg.
    pub ffmpeg: String,
    /// Full first version line reported by ffprobe.
    pub ffprobe: String,
    /// H.264 encoder selected from the verified FFmpeg capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_encoder: Option<String>,
}

/// Return the startup-verified sanitizer versions for private audit records.
pub fn tool_versions() -> Option<&'static ToolVersions> {
    TOOL_VERSIONS.get()
}

/// High-level class used for route enforcement and bounded metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaClass {
    Image,
    Video,
    Audio,
}

impl MediaClass {
    /// Stable, bounded metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

/// Content-derived media information. Request MIME types and filenames are
/// never used to construct this value.
#[derive(Debug, Clone)]
pub struct MediaProbe {
    pub class: MediaClass,
    pub mime: String,
    pub ext: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_secs: Option<f64>,
    pub frame_count: Option<u64>,
}

/// A verified artifact ready for content-addressed publication.
pub struct SanitizedMedia {
    pub file: NamedTempFile,
    pub probe: MediaProbe,
}

/// Verify required executables at relay startup. This intentionally fails
/// closed: a deployment without its privacy controls must not accept uploads.
pub async fn validate_toolchain(config: &MediaConfig) -> Result<(), MediaError> {
    let exiftool = successful_version(&config.exiftool_path, "-ver").await?;
    let ffmpeg = successful_version(&config.ffmpeg_path, "-version").await?;
    let ffprobe = successful_version(&config.ffprobe_path, "-version").await?;
    reject_nonredistributable_build(&config.ffmpeg_path).await?;
    reject_nonredistributable_build(&config.ffprobe_path).await?;
    let encoders = run_tool(
        &config.ffmpeg_path,
        &["-hide_banner", "-encoders"],
        Duration::from_secs(15),
    )
    .await?;
    let encoders = String::from_utf8_lossy(&encoders.stdout);
    let video_encoder = select_video_encoder(&encoders).ok_or(MediaError::ToolUnavailable)?;
    for required in ["aac"] {
        if !encoders.contains(required) {
            return Err(MediaError::ToolUnavailable);
        }
    }
    let decoders = run_tool(
        &config.ffmpeg_path,
        &["-hide_banner", "-decoders"],
        Duration::from_secs(15),
    )
    .await?;
    let decoders = String::from_utf8_lossy(&decoders.stdout);
    for required in [
        "h264",
        "hevc",
        "vp8",
        "vp9",
        "av1",
        "aac",
        "mp3",
        "flac",
        "vorbis",
        "opus",
        "pcm_s16le",
    ] {
        if !decoders.contains(required) {
            return Err(MediaError::ToolUnavailable);
        }
    }
    let _ = TOOL_VERSIONS.set(ToolVersions {
        exiftool,
        ffmpeg,
        ffprobe,
        video_encoder: Some(video_encoder.to_string()),
    });
    Ok(())
}

async fn reject_nonredistributable_build(program: &str) -> Result<(), MediaError> {
    let output = run_tool(
        program,
        &["-hide_banner", "-buildconf"],
        Duration::from_secs(15),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::ToolUnavailable);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if build_is_nonredistributable(&stdout) || build_is_nonredistributable(&stderr) {
        return Err(MediaError::ToolUnavailable);
    }
    Ok(())
}

fn build_is_nonredistributable(configuration: &str) -> bool {
    configuration
        .split_ascii_whitespace()
        .any(|argument| argument == "--enable-nonfree")
}

fn select_video_encoder(encoders: &str) -> Option<&'static str> {
    ["libopenh264", "libx264"].into_iter().find(|encoder| {
        encoders
            .split_ascii_whitespace()
            .any(|word| word == *encoder)
    })
}

fn video_encoder() -> Result<&'static str, MediaError> {
    TOOL_VERSIONS
        .get()
        .and_then(|versions| versions.video_encoder.as_deref())
        .ok_or(MediaError::ToolUnavailable)
}

async fn successful_version(program: &str, arg: &str) -> Result<String, MediaError> {
    let output = run_tool(program, &[arg], Duration::from_secs(15)).await?;
    if !output.status.success() {
        return Err(MediaError::ToolUnavailable);
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or(MediaError::ToolUnavailable)
}

/// Return `None` for a non-media attachment, a supported probe for accepted
/// media, or `UnsupportedMedia` when a parser recognizes media outside the
/// compliance allowlist.
pub async fn probe_media(
    path: &Path,
    sniff: &[u8],
    config: &MediaConfig,
) -> Result<Option<MediaProbe>, MediaError> {
    if let Some((mime, ext)) = iso_bmff_still_image(sniff) {
        let mut probe = match probe_with_ffprobe(path, config).await {
            Ok(probe) => probe,
            Err(_) => {
                let (width, height, frame_count) = exiftool_image_dimensions(path, config).await?;
                MediaProbe {
                    class: MediaClass::Image,
                    mime: mime.to_string(),
                    ext: ext.to_string(),
                    video_codec: None,
                    audio_codec: None,
                    width: Some(width),
                    height: Some(height),
                    duration_secs: None,
                    frame_count,
                }
            }
        };
        probe.class = MediaClass::Image;
        probe.mime = mime.to_string();
        probe.ext = ext.to_string();
        validate_media_limits(&probe)?;
        return Ok(Some(probe));
    }
    let mut recognized_media = false;
    if let Some(kind) = infer::get(sniff) {
        if let Some((mime, ext)) = supported_image(kind.mime_type()) {
            let mut probe = match probe_with_ffprobe(path, config).await {
                Ok(probe) => probe,
                Err(_) => {
                    let (width, height) = image_dimensions(path)?;
                    MediaProbe {
                        class: MediaClass::Image,
                        mime: mime.to_string(),
                        ext: ext.to_string(),
                        video_codec: None,
                        audio_codec: None,
                        width: Some(width),
                        height: Some(height),
                        duration_secs: None,
                        frame_count: None,
                    }
                }
            };
            probe.class = MediaClass::Image;
            probe.mime = mime.to_string();
            probe.ext = ext.to_string();
            validate_media_limits(&probe)?;
            return Ok(Some(probe));
        }
        if kind.mime_type().starts_with("image/") {
            return Err(MediaError::UnsupportedMedia(kind.mime_type().to_string()));
        }
        recognized_media =
            kind.mime_type().starts_with("video/") || kind.mime_type().starts_with("audio/");
    }

    let probe = match probe_with_ffprobe(path, config).await {
        Ok(probe) => probe,
        Err(MediaError::SanitizationFailed) if !recognized_media => return Ok(None),
        Err(error) => return Err(error),
    };
    match probe.class {
        MediaClass::Video => {
            if !matches!(probe.ext.as_str(), "mp4" | "mov" | "webm" | "mkv") {
                return Err(MediaError::UnsupportedMedia(probe.mime));
            }
        }
        MediaClass::Audio => {
            if !matches!(
                probe.ext.as_str(),
                "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus"
            ) {
                return Err(MediaError::UnsupportedMedia(probe.mime));
            }
        }
        MediaClass::Image => {}
    }
    validate_media_limits(&probe)?;
    Ok(Some(probe))
}

fn image_dimensions(path: &Path) -> Result<(u32, u32), MediaError> {
    let dimensions = imagesize::size(path).map_err(|_| MediaError::SanitizationFailed)?;
    let width = u32::try_from(dimensions.width).map_err(|_| MediaError::ImageTooLarge)?;
    let height = u32::try_from(dimensions.height).map_err(|_| MediaError::ImageTooLarge)?;
    if width == 0 || height == 0 {
        return Err(MediaError::SanitizationFailed);
    }
    Ok((width, height))
}

async fn exiftool_image_dimensions(
    path: &Path,
    config: &MediaConfig,
) -> Result<(u32, u32, Option<u64>), MediaError> {
    let args = [
        "-j".to_string(),
        "-n".to_string(),
        "-ImageWidth".to_string(),
        "-ImageHeight".to_string(),
        "-FrameCount".to_string(),
        "-ImageCount".to_string(),
        path_string(path),
    ];
    let output = run_tool(
        &config.exiftool_path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(config.image_process_timeout_secs),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::SanitizationFailed);
    }
    let documents: Vec<Value> =
        serde_json::from_slice(&output.stdout).map_err(|_| MediaError::SanitizationFailed)?;
    let document = documents
        .first()
        .and_then(Value::as_object)
        .ok_or(MediaError::SanitizationFailed)?;
    let parse_u64 = |name: &str| {
        document.get(name).and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
    };
    let width = parse_u64("ImageWidth")
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(MediaError::SanitizationFailed)?;
    let height = parse_u64("ImageHeight")
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(MediaError::SanitizationFailed)?;
    let frame_count = parse_u64("FrameCount").or_else(|| parse_u64("ImageCount"));
    Ok((width, height, frame_count))
}

/// Remove metadata and return a separately verified output artifact.
pub async fn sanitize(
    source: &Path,
    source_probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<SanitizedMedia, MediaError> {
    let (file, expected_class) = match source_probe.class {
        MediaClass::Image => (
            sanitize_image(source, source_probe, config).await?,
            MediaClass::Image,
        ),
        MediaClass::Video => (
            sanitize_video(source, source_probe, config).await?,
            MediaClass::Video,
        ),
        MediaClass::Audio => (
            sanitize_audio(source, source_probe, config).await?,
            MediaClass::Audio,
        ),
    };

    verify_forbidden_metadata(file.path(), config).await?;
    let sniff = read_sniff(file.path()).await?;
    let output_probe = probe_media(file.path(), &sniff, config)
        .await?
        .ok_or(MediaError::SanitizationFailed)?;
    if output_probe.class != expected_class {
        return Err(MediaError::SanitizationFailed);
    }
    verify_stream_shape(&output_probe, file.path(), config).await?;
    Ok(SanitizedMedia {
        file,
        probe: output_probe,
    })
}

async fn sanitize_image(
    source: &Path,
    probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<NamedTempFile, MediaError> {
    if probe.ext == "bmp" {
        let output = temp_with_suffix(".png")?;
        let args = vec![
            "-nostdin".to_string(),
            "-v".to_string(),
            "error".to_string(),
            "-y".to_string(),
            "-i".to_string(),
            path_string(source),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map_metadata".to_string(),
            "-1".to_string(),
            "-frames:v".to_string(),
            "1".to_string(),
            path_string(output.path()),
        ];
        require_success(
            &config.ffmpeg_path,
            &args,
            Duration::from_secs(config.image_process_timeout_secs),
        )
        .await?;
        return Ok(output);
    }

    if probe.ext == "tiff" {
        let output = temp_with_suffix(".tiff")?;
        let args = vec![
            "-nostdin".to_string(),
            "-v".to_string(),
            "error".to_string(),
            "-y".to_string(),
            "-i".to_string(),
            path_string(source),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map_metadata".to_string(),
            "-1".to_string(),
            "-frames:v".to_string(),
            "1".to_string(),
            "-c:v".to_string(),
            "tiff".to_string(),
            path_string(output.path()),
        ];
        require_success(
            &config.ffmpeg_path,
            &args,
            Duration::from_secs(config.image_process_timeout_secs),
        )
        .await?;
        let args = [
            "-overwrite_original".to_string(),
            "-tagsFromFile".to_string(),
            path_string(source),
            "-ICC_Profile:All".to_string(),
            "-ColorSpaceTags".to_string(),
            "-Orientation".to_string(),
            "-Software=".to_string(),
            path_string(output.path()),
        ];
        require_success(
            &config.exiftool_path,
            &args,
            Duration::from_secs(config.image_process_timeout_secs),
        )
        .await?;
        return Ok(output);
    }

    let output = temp_with_suffix(&format!(".{}", probe.ext))?;
    tokio::fs::copy(source, output.path())
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    // Delete everything, excluding the ICC block from deletion, then copy back
    // only rendering-critical color-space and orientation tags from the file.
    let args = [
        "-overwrite_original".to_string(),
        "-all=".to_string(),
        "--ICC_Profile:All".to_string(),
        "-tagsFromFile".to_string(),
        "@".to_string(),
        "-ColorSpaceTags".to_string(),
        "-Orientation".to_string(),
        path_string(output.path()),
    ];
    require_success(
        &config.exiftool_path,
        &args,
        Duration::from_secs(config.image_process_timeout_secs),
    )
    .await?;
    Ok(output)
}

async fn sanitize_video(
    source: &Path,
    probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<NamedTempFile, MediaError> {
    let output = temp_with_suffix(".mp4")?;
    let can_copy = probe.video_codec.as_deref() == Some("h264")
        && matches!(probe.audio_codec.as_deref(), None | Some("aac"));
    let mut args = common_ffmpeg_input(source);
    args.extend(strings(&[
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-map_metadata",
        "-1",
        "-map_metadata:s",
        "-1",
        "-map_metadata:c",
        "-1",
        "-map_metadata:p",
        "-1",
        "-map_chapters",
        "-1",
        "-sn",
        "-dn",
    ]));
    if can_copy {
        args.extend(strings(&["-c:v", "copy", "-c:a", "copy"]));
    } else {
        match video_encoder()? {
            "libopenh264" => args.extend(strings(&[
                "-c:v",
                "libopenh264",
                "-b:v",
                "4M",
                "-maxrate",
                "4M",
                "-bufsize",
                "8M",
                "-pix_fmt",
                "yuv420p",
            ])),
            "libx264" => args.extend(strings(&[
                "-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p",
            ])),
            _ => return Err(MediaError::ToolUnavailable),
        }
        args.extend(strings(&["-c:a", "aac", "-b:a", "192k"]));
    }
    args.extend(strings(&[
        "-movflags",
        "+faststart",
        "-metadata",
        "encoder=",
    ]));
    args.push(path_string(output.path()));
    require_success(
        &config.ffmpeg_path,
        &args,
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    Ok(output)
}

async fn sanitize_audio(
    source: &Path,
    probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<NamedTempFile, MediaError> {
    let (suffix, format) = match probe.ext.as_str() {
        "mp3" => (".mp3", "mp3"),
        "m4a" => (".m4a", "mp4"),
        "aac" => (".aac", "adts"),
        "flac" => (".flac", "flac"),
        "wav" => (".wav", "wav"),
        "ogg" => (".ogg", "ogg"),
        "opus" => (".opus", "opus"),
        _ => return Err(MediaError::UnsupportedMedia(probe.mime.clone())),
    };
    let output = temp_with_suffix(suffix)?;
    let mut args = common_ffmpeg_input(source);
    args.extend(strings(&[
        "-map",
        "0:a:0",
        "-map_metadata",
        "-1",
        "-map_metadata:s",
        "-1",
        "-map_metadata:c",
        "-1",
        "-map_metadata:p",
        "-1",
        "-map_chapters",
        "-1",
        "-vn",
        "-sn",
        "-dn",
        "-c:a",
        "copy",
        "-metadata",
        "encoder=",
        "-f",
        format,
    ]));
    if probe.ext == "mp3" {
        args.extend(strings(&["-id3v2_version", "0", "-write_id3v1", "0"]));
    }
    if probe.ext == "wav" {
        args.extend(strings(&["-fflags", "+bitexact", "-flags:a", "+bitexact"]));
    }
    args.push(path_string(output.path()));
    require_success(
        &config.ffmpeg_path,
        &args,
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    Ok(output)
}

async fn verify_forbidden_metadata(path: &Path, config: &MediaConfig) -> Result<(), MediaError> {
    let selectors = [
        "-GPS*",
        "-Location*",
        "-*Latitude*",
        "-*Longitude*",
        "-*Altitude*",
        "-MakerNotes:All",
        "-Make",
        "-Model",
        "-*SerialNumber*",
        "-OwnerName",
        "-Artist",
        "-Author",
        "-Creator",
        "-Comment",
        "-Description",
        "-Software",
        "-DateTimeOriginal",
        "-CreateDate",
        "-ModifyDate",
        "-MediaCreateDate",
        "-TrackCreateDate",
        "-XMPToolkit",
        "-ThumbnailImage",
        "-PreviewImage",
        "-History*",
        "-DocumentID",
        "-InstanceID",
        "-Lens*",
        "-Camera*",
        "-Copyright",
        "-Title",
        "-Keywords",
        "-Subject",
        "-UserDefinedText",
    ];
    let mut args = strings(&["-api", "LargeFileSupport=1", "-ee", "-j", "-G1", "-s"]);
    args.extend(selectors.iter().map(|value| (*value).to_string()));
    args.push(path_string(path));
    let output = run_tool(
        &config.exiftool_path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::SanitizationFailed);
    }
    let documents: Vec<Value> =
        serde_json::from_slice(&output.stdout).map_err(|_| MediaError::SanitizationFailed)?;
    let has_forbidden = documents.iter().any(|document| {
        document
            .as_object()
            .map(|object| {
                object.iter().any(|(key, value)| {
                    if key.ends_with("SourceFile") {
                        return false;
                    }
                    let is_date = key.to_ascii_lowercase().contains("date");
                    let is_zero_date = value.as_str().is_some_and(|value| {
                        value.starts_with("0000:00:00") || value.starts_with("1904:01:01")
                    });
                    !is_date || !is_zero_date
                })
            })
            .unwrap_or(true)
    });
    if has_forbidden {
        return Err(MediaError::ResidualMetadata);
    }
    Ok(())
}

async fn verify_stream_shape(
    probe: &MediaProbe,
    path: &Path,
    config: &MediaConfig,
) -> Result<(), MediaError> {
    // Image structure and geometry/frame limits were already checked by
    // `probe_media`. HEIC/HEIF support is supplied by ExifTool on deployments
    // where FFmpeg cannot expose these still-image containers as streams.
    if probe.class == MediaClass::Image {
        return Ok(());
    }
    let json = ffprobe_json(path, config).await?;
    let streams = json["streams"]
        .as_array()
        .ok_or(MediaError::SanitizationFailed)?;
    let video_count = streams
        .iter()
        .filter(|stream| stream["codec_type"] == "video")
        .count();
    let audio_count = streams
        .iter()
        .filter(|stream| stream["codec_type"] == "audio")
        .count();
    let unexpected = streams.iter().any(|stream| {
        !matches!(stream["codec_type"].as_str(), Some("video" | "audio"))
            || stream
                .get("tags")
                .and_then(Value::as_object)
                .is_some_and(|tags| tags.keys().any(|key| !is_structural_tag(key)))
    });
    let valid = match probe.class {
        MediaClass::Image => video_count == 1 && audio_count == 0,
        MediaClass::Video => video_count == 1 && audio_count <= 1,
        MediaClass::Audio => video_count == 0 && audio_count == 1,
    };
    if unexpected || !valid {
        return Err(MediaError::ResidualMetadata);
    }
    Ok(())
}

fn is_structural_tag(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "language" | "handler_name" | "vendor_id" | "encoder"
    )
}

async fn probe_with_ffprobe(path: &Path, config: &MediaConfig) -> Result<MediaProbe, MediaError> {
    let json = ffprobe_json(path, config).await?;
    let streams = json["streams"]
        .as_array()
        .ok_or(MediaError::SanitizationFailed)?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video");
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio");
    let format_name = json["format"]["format_name"].as_str().unwrap_or_default();
    let video_codec = video
        .and_then(|stream| stream["codec_name"].as_str())
        .map(str::to_string);
    let audio_codec = audio
        .and_then(|stream| stream["codec_name"].as_str())
        .map(str::to_string);

    let (class, mime, ext) = if video.is_some() {
        if is_still_image_format(format_name) {
            let (mime, ext) = image_format(format_name, video_codec.as_deref())?;
            (MediaClass::Image, mime, ext)
        } else {
            let (mime, ext) = video_format(format_name)?;
            (MediaClass::Video, mime, ext)
        }
    } else if audio.is_some() {
        let (mime, ext) = audio_format(format_name, audio_codec.as_deref())?;
        (MediaClass::Audio, mime, ext)
    } else {
        return Err(MediaError::SanitizationFailed);
    };

    let width = video
        .and_then(|stream| stream["width"].as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let height = video
        .and_then(|stream| stream["height"].as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let duration_secs = json["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            video
                .or(audio)
                .and_then(|stream| stream["duration"].as_str())
                .and_then(|value| value.parse().ok())
        });
    let frame_count = video
        .and_then(|stream| stream["nb_frames"].as_str())
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            video
                .and_then(|stream| stream["nb_read_frames"].as_str())
                .and_then(|value| value.parse().ok())
        });
    Ok(MediaProbe {
        class,
        mime: mime.to_string(),
        ext: ext.to_string(),
        video_codec,
        audio_codec,
        width,
        height,
        duration_secs,
        frame_count,
    })
}

async fn ffprobe_json(path: &Path, config: &MediaConfig) -> Result<Value, MediaError> {
    let args = [
        "-v".to_string(),
        "error".to_string(),
        "-count_frames".to_string(),
        "-show_streams".to_string(),
        "-show_format".to_string(),
        "-of".to_string(),
        "json".to_string(),
        path_string(path),
    ];
    let output = run_tool(
        &config.ffprobe_path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(MediaError::SanitizationFailed);
    }
    serde_json::from_slice(&output.stdout).map_err(|_| MediaError::SanitizationFailed)
}

fn validate_media_limits(probe: &MediaProbe) -> Result<(), MediaError> {
    match probe.class {
        MediaClass::Image => {
            if let (Some(width), Some(height)) = (probe.width, probe.height) {
                let pixels = u64::from(width) * u64::from(height);
                if pixels > MAX_IMAGE_PIXELS {
                    return Err(MediaError::ImageTooLarge);
                }
                if probe.frame_count.is_some_and(|frames| {
                    frames > MAX_ANIMATION_FRAMES
                        || pixels.saturating_mul(frames) > MAX_ANIMATION_PIXELS
                }) {
                    return Err(MediaError::ImageTooLarge);
                }
            }
        }
        MediaClass::Video => {
            if probe
                .duration_secs
                .is_none_or(|duration| duration <= 0.0 || duration > MAX_VIDEO_DURATION_SECS)
            {
                return Err(MediaError::DurationTooLong);
            }
            if probe.width.is_none_or(|width| width > MAX_VIDEO_WIDTH)
                || probe.height.is_none_or(|height| height > MAX_VIDEO_HEIGHT)
            {
                return Err(MediaError::ResolutionTooHigh);
            }
        }
        MediaClass::Audio => {}
    }
    Ok(())
}

fn iso_bmff_still_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    let upper = bytes.len().min(64);
    (8..upper.saturating_sub(3))
        .step_by(4)
        .find_map(|offset| match &bytes[offset..offset + 4] {
            b"avif" | b"avis" => Some(("image/avif", "avif")),
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
                Some(("image/heic", "heic"))
            }
            _ => None,
        })
}

fn supported_image(mime: &str) -> Option<(&'static str, &'static str)> {
    match mime {
        "image/jpeg" => Some(("image/jpeg", "jpg")),
        "image/png" => Some(("image/png", "png")),
        "image/gif" => Some(("image/gif", "gif")),
        "image/webp" => Some(("image/webp", "webp")),
        "image/tiff" => Some(("image/tiff", "tiff")),
        "image/bmp" | "image/x-ms-bmp" => Some(("image/bmp", "bmp")),
        "image/heic" | "image/heif" => Some(("image/heic", "heic")),
        "image/avif" => Some(("image/avif", "avif")),
        _ => None,
    }
}

fn is_still_image_format(format: &str) -> bool {
    format.split(',').any(|name| {
        matches!(
            name,
            "image2"
                | "jpeg_pipe"
                | "png_pipe"
                | "gif"
                | "webp_pipe"
                | "tiff_pipe"
                | "bmp_pipe"
                | "avif"
                | "heif"
        )
    })
}

fn image_format(
    format: &str,
    codec: Option<&str>,
) -> Result<(&'static str, &'static str), MediaError> {
    if let Some(pair) = codec.and_then(|codec| match codec {
        "mjpeg" => Some(("image/jpeg", "jpg")),
        "png" => Some(("image/png", "png")),
        "gif" => Some(("image/gif", "gif")),
        "webp" => Some(("image/webp", "webp")),
        "tiff" => Some(("image/tiff", "tiff")),
        "bmp" => Some(("image/bmp", "bmp")),
        "av1" => Some(("image/avif", "avif")),
        _ => None,
    }) {
        return Ok(pair);
    }
    Err(MediaError::UnsupportedMedia(format.to_string()))
}

fn video_format(format: &str) -> Result<(&'static str, &'static str), MediaError> {
    if format.contains("matroska") || format.contains("webm") {
        if format.contains("webm") {
            Ok(("video/webm", "webm"))
        } else {
            Ok(("video/x-matroska", "mkv"))
        }
    } else if format.contains("mov") || format.contains("mp4") {
        Ok(("video/mp4", "mp4"))
    } else {
        Err(MediaError::UnsupportedMedia(format.to_string()))
    }
}

fn audio_format(
    format: &str,
    codec: Option<&str>,
) -> Result<(&'static str, &'static str), MediaError> {
    if format.contains("mp3") {
        Ok(("audio/mpeg", "mp3"))
    } else if format.contains("mov") || format.contains("mp4") {
        Ok(("audio/mp4", "m4a"))
    } else if format.contains("aac") {
        Ok(("audio/aac", "aac"))
    } else if format.contains("flac") {
        Ok(("audio/flac", "flac"))
    } else if format.contains("wav") {
        Ok(("audio/wav", "wav"))
    } else if format.contains("ogg") {
        if codec == Some("opus") {
            Ok(("audio/opus", "opus"))
        } else {
            Ok(("audio/ogg", "ogg"))
        }
    } else {
        Err(MediaError::UnsupportedMedia(format.to_string()))
    }
}

fn common_ffmpeg_input(source: &Path) -> Vec<String> {
    let mut args = strings(&["-nostdin", "-v", "error", "-y", "-i"]);
    args.push(path_string(source));
    args
}

async fn require_success(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<(), MediaError> {
    let output = run_tool(
        program,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        timeout,
    )
    .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(MediaError::SanitizationFailed)
    }
}

async fn run_tool(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<std::process::Output, MediaError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            MediaError::ToolUnavailable
        } else {
            MediaError::Io(error.to_string())
        }
    })?;
    let stdout = child.stdout.take().ok_or(MediaError::Internal)?;
    let stderr = child.stderr.take().ok_or(MediaError::Internal)?;
    let stdout_task = tokio::spawn(read_bounded_output(stdout));
    let stderr_task = tokio::spawn(read_bounded_output(stderr));
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(result) => result.map_err(|error| MediaError::Io(error.to_string()))?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(MediaError::SanitizationFailed);
        }
    };
    let stdout = stdout_task.await.map_err(|_| MediaError::Internal)??;
    let stderr = stderr_task.await.map_err(|_| MediaError::Internal)??;
    if stdout.len() > MAX_TOOL_OUTPUT || stderr.len() > MAX_TOOL_OUTPUT {
        return Err(MediaError::SanitizationFailed);
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

async fn read_bounded_output(
    reader: impl tokio::io::AsyncRead + Unpin,
) -> Result<Vec<u8>, MediaError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_TOOL_OUTPUT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    Ok(bytes)
}

async fn read_sniff(path: &Path) -> Result<Vec<u8>, MediaError> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    let mut bytes = vec![0_u8; 4096];
    let read = file
        .read(&mut bytes)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    bytes.truncate(read);
    Ok(bytes)
}

fn temp_with_suffix(suffix: &str) -> Result<NamedTempFile, MediaError> {
    Builder::new()
        .prefix("buzz-sanitized-")
        .suffix(suffix)
        .tempfile()
        .map_err(|error| MediaError::Io(error.to_string()))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_format_tables_are_bounded() {
        assert_eq!(supported_image("image/jpeg"), Some(("image/jpeg", "jpg")));
        assert!(supported_image("image/svg+xml").is_none());
        assert_eq!(audio_format("ogg", Some("opus")).unwrap().1, "opus");
        assert!(video_format("avi").is_err());
        assert_eq!(
            iso_bmff_still_image(b"\0\0\0\x18ftypheic\0\0\0\0heic"),
            Some(("image/heic", "heic"))
        );
    }

    #[test]
    fn structural_tag_allowlist_is_narrow() {
        assert!(is_structural_tag("language"));
        assert!(!is_structural_tag("location"));
        assert!(!is_structural_tag("title"));
    }

    #[test]
    fn nonredistributable_ffmpeg_builds_are_rejected() {
        assert!(build_is_nonredistributable(
            "configuration: --enable-gpl --enable-nonfree --enable-libx264"
        ));
        assert!(!build_is_nonredistributable(
            "configuration: --disable-autodetect --enable-libopenh264 --enable-shared"
        ));
    }

    #[test]
    fn openh264_is_preferred_without_requiring_it_from_operators() {
        let both = " V....D libx264 H.264 / AVC\n V....D libopenh264 OpenH264 H.264";
        assert_eq!(select_video_encoder(both), Some("libopenh264"));
        assert_eq!(
            select_video_encoder(" V....D libx264 H.264 / AVC"),
            Some("libx264")
        );
        assert_eq!(select_video_encoder(" A..... aac AAC"), None);
    }

    #[test]
    fn video_and_animation_limits_fail_closed() {
        let mut probe = MediaProbe {
            class: MediaClass::Video,
            mime: "video/mp4".to_string(),
            ext: "mp4".to_string(),
            video_codec: Some("h264".to_string()),
            audio_codec: Some("aac".to_string()),
            width: Some(3840),
            height: Some(2160),
            duration_secs: Some(600.0),
            frame_count: Some(18_000),
        };
        assert!(validate_media_limits(&probe).is_ok());
        probe.duration_secs = Some(600.1);
        assert!(matches!(
            validate_media_limits(&probe),
            Err(MediaError::DurationTooLong)
        ));
        probe.class = MediaClass::Image;
        probe.width = Some(1_000);
        probe.height = Some(1_000);
        probe.duration_secs = None;
        probe.frame_count = Some(1_001);
        assert!(matches!(
            validate_media_limits(&probe),
            Err(MediaError::ImageTooLarge)
        ));
    }
}
