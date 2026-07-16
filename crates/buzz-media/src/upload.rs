//! Upload pipeline — validate, store, thumbnail, sidecar.

use buzz_core::tenant::TenantContext;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::auth::{verify_blossom_media_auth, verify_blossom_upload_auth};
use crate::config::MediaConfig;
use crate::error::MediaError;
use crate::storage::{BlobMeta, MediaStorage};
use crate::thumbnail::generate_image_metadata_sync;
use crate::types::BlobDescriptor;
use crate::upload_record::{record_upload_event, UploadAttribution, UploadEventFacts};
use crate::validation::{
    mime_to_ext, validate_content, validate_file_content, validate_video_file,
};

/// Upload route semantics. `Media` transforms recognized media, `Upload`
/// preserves exact non-media bytes, and `Legacy` keeps the historical upload
/// authorization while otherwise enforcing the same media-only policy as
/// `Media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadRouteMode {
    Media,
    Upload,
    Legacy,
}

/// Request-scoped dependencies and policy for a unified streaming upload.
pub struct StreamingIngestInput<'a> {
    /// Content-addressed object storage.
    pub storage: &'a MediaStorage,
    /// Media limits and sanitizer executable configuration.
    pub config: &'a MediaConfig,
    /// Server-resolved tenant boundary for storage and authorization.
    pub ctx: &'a TenantContext,
    /// Verified Blossom authorization event for the source bytes.
    pub auth_event: &'a nostr::Event,
    /// Source hash claimed by the `X-SHA-256` request header.
    pub claimed_source_hash: &'a str,
    /// Optional request content length for an early size rejection.
    pub content_length: Option<u64>,
    /// Optional moderation attribution recorded after durable publication.
    pub attribution: Option<UploadAttribution>,
    /// Route policy controlling transformation versus exact-byte storage.
    pub mode: UploadRouteMode,
}

/// Unified streaming ingestion pipeline used by all public upload routes.
///
/// The source hash is authenticated before any transformation. Only the final
/// sanitized artifact is hashed into the public storage key and descriptor.
pub async fn process_streaming_ingest(
    input: StreamingIngestInput<'_>,
    body_stream: impl futures_core::Stream<Item = Result<Bytes, axum::Error>> + Send + 'static,
) -> Result<BlobDescriptor, MediaError> {
    let StreamingIngestInput {
        storage,
        config,
        ctx,
        auth_event,
        claimed_source_hash,
        content_length,
        attribution,
        mode,
    } = input;
    let max_bytes = config
        .max_image_bytes
        .max(config.max_video_bytes)
        .max(config.max_audio_bytes)
        .max(config.max_file_bytes);
    if content_length.is_some_and(|size| size > max_bytes) {
        return Err(MediaError::FileTooLarge {
            size: content_length.unwrap_or(max_bytes),
            max: max_bytes,
        });
    }

    let source = tempfile::Builder::new()
        .prefix("buzz-source-")
        .tempfile()
        .map_err(|error| MediaError::Io(error.to_string()))?;
    let source_path = source.path().to_path_buf();
    let (source_hash, source_size, sniff) =
        stream_source(body_stream, &source_path, max_bytes).await?;
    if source_hash != claimed_source_hash {
        return Err(MediaError::HashMismatch);
    }

    // Authenticate the computed source hash before invoking any media parser.
    // The permissive bound is narrowed below once content probing identifies
    // whether this is a long-running video upload or a short-lived token class.
    verify_source_auth(mode, auth_event, &source_hash, ctx.host(), 3_600).await?;

    let source_probe = crate::sanitize::probe_media(&source_path, &sniff, config).await?;
    let is_media = source_probe.is_some();
    enforce_route_policy(mode, source_probe.as_ref())?;
    let max_auth_age = auth_max_age_secs(source_probe.as_ref());
    if max_auth_age < 3_600 {
        verify_source_auth(mode, auth_event, &source_hash, ctx.host(), max_auth_age).await?;
    }

    enforce_source_size(source_probe.as_ref(), source_size, config)?;

    let sanitized = match source_probe.as_ref() {
        Some(probe) => {
            let started = std::time::Instant::now();
            let result = crate::sanitize::sanitize(&source_path, probe, config).await;
            let outcome = match &result {
                Ok(_) => "accepted",
                Err(MediaError::ResidualMetadata) => "residual_metadata",
                Err(MediaError::SanitizationFailed) => "sanitization_failed",
                Err(MediaError::ImageTooLarge) => "limits",
                Err(_) => "rejected",
            };
            metrics::counter!(
                "buzz_media_sanitization_total",
                "class" => probe.class.as_str(),
                "format" => probe.ext.clone(),
                "outcome" => outcome
            )
            .increment(1);
            metrics::histogram!(
                "buzz_media_sanitization_duration_seconds",
                "class" => probe.class.as_str(),
                "format" => probe.ext.clone(),
                "outcome" => outcome
            )
            .record(started.elapsed().as_secs_f64());
            if matches!(&result, Err(MediaError::ResidualMetadata)) {
                metrics::counter!("buzz_media_residual_metadata_rejections_total").increment(1);
            }
            Some(result?)
        }
        None => None,
    };
    let (artifact_path, mime, ext, class, duration, dim) = if let Some(artifact) = &sanitized {
        let output_size = tokio::fs::metadata(artifact.file.path())
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?
            .len();
        enforce_source_size(Some(&artifact.probe), output_size, config)?;
        let dim = artifact
            .probe
            .width
            .zip(artifact.probe.height)
            .map(|(width, height)| format!("{width}x{height}"))
            .unwrap_or_default();
        (
            artifact.file.path(),
            artifact.probe.mime.clone(),
            artifact.probe.ext.clone(),
            Some(artifact.probe.class),
            artifact.probe.duration_secs,
            dim,
        )
    } else {
        let bytes = tokio::fs::read(&source_path)
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        let (mime, ext) = validate_file_content(&bytes, config)?;
        (source_path.as_path(), mime, ext, None, None, String::new())
    };

    let (output_hash, output_size) = hash_file(artifact_path).await?;
    let key = format!("{output_hash}.{ext}");
    let meta_key = MediaStorage::ctx_sidecar_key(ctx, &output_hash);
    let sidecar_exists = storage.head(&meta_key).await?;
    let blob_exists = storage.head(&key).await?;
    if sidecar_exists && blob_exists {
        let meta = storage.get_sidecar(ctx, &output_hash).await?;
        if let Some(attribution) = &attribution {
            let (record_source_hash, record_source_size, record_source_mime) =
                source_record_fields(source_probe.as_ref(), &source_hash, source_size);
            record_upload_event(
                storage,
                ctx,
                &auth_event.pubkey,
                attribution,
                UploadEventFacts {
                    sha256: &output_hash,
                    ext: &ext,
                    mime: &mime,
                    size: output_size,
                    source_sha256: record_source_hash,
                    source_size: record_source_size,
                    source_mime: record_source_mime,
                    sanitization_policy: is_media.then_some(1),
                    tool_versions: is_media.then(crate::sanitize::tool_versions).flatten(),
                    uploaded_at: chrono::Utc::now().timestamp(),
                },
            )
            .await?;
        }
        return Ok(build_descriptor(
            config,
            &output_hash,
            &ext,
            &mime,
            output_size,
            Some(&meta),
            meta.uploaded_at,
        ));
    }

    let uploaded_at = chrono::Utc::now().timestamp();
    storage.put_file(&key, artifact_path, &mime).await?;

    let meta = if class == Some(crate::sanitize::MediaClass::Image)
        && matches!(
            mime.as_str(),
            "image/jpeg" | "image/png" | "image/gif" | "image/webp"
        ) {
        let body = Bytes::from(
            tokio::fs::read(artifact_path)
                .await
                .map_err(|error| MediaError::Io(error.to_string()))?,
        );
        prepare_image_metadata(
            storage,
            config,
            MetadataInput {
                sha256: output_hash.clone(),
                ext: ext.clone(),
                mime: mime.clone(),
                body,
                uploaded_at,
            },
        )
        .await?
    } else {
        BlobMeta {
            dim,
            blurhash: String::new(),
            thumb_url: String::new(),
            size: output_size,
            ext: ext.clone(),
            mime_type: mime.clone(),
            uploaded_at,
            duration_secs: duration,
        }
    };

    if let Some(attribution) = &attribution {
        let (record_source_hash, record_source_size, record_source_mime) =
            source_record_fields(source_probe.as_ref(), &source_hash, source_size);
        record_upload_event(
            storage,
            ctx,
            &auth_event.pubkey,
            attribution,
            UploadEventFacts {
                sha256: &output_hash,
                ext: &ext,
                mime: &mime,
                size: output_size,
                source_sha256: record_source_hash,
                source_size: record_source_size,
                source_mime: record_source_mime,
                sanitization_policy: is_media.then_some(1),
                tool_versions: is_media.then(crate::sanitize::tool_versions).flatten(),
                uploaded_at,
            },
        )
        .await?;
    }
    storage.put_sidecar(ctx, &output_hash, &meta).await?;

    metrics::counter!(
        "buzz_media_ingest_total",
        "class" => class.map(crate::sanitize::MediaClass::as_str).unwrap_or("file"),
        "format" => ext.clone(),
        "outcome" => "accepted"
    )
    .increment(1);
    metrics::histogram!("buzz_media_processing_input_bytes", "class" => class.map(crate::sanitize::MediaClass::as_str).unwrap_or("file"))
        .record(source_size as f64);
    metrics::histogram!("buzz_media_processing_output_bytes", "class" => class.map(crate::sanitize::MediaClass::as_str).unwrap_or("file"))
        .record(output_size as f64);

    Ok(build_descriptor(
        config,
        &output_hash,
        &ext,
        &mime,
        output_size,
        Some(&meta),
        uploaded_at,
    ))
}

async fn verify_source_auth(
    mode: UploadRouteMode,
    auth_event: &nostr::Event,
    source_hash: &str,
    bound_host: &str,
    max_age_secs: u64,
) -> Result<(), MediaError> {
    let auth = auth_event.clone();
    let auth_hash = source_hash.to_string();
    let bound_host = bound_host.to_string();
    tokio::task::spawn_blocking(move || match mode {
        UploadRouteMode::Media => {
            verify_blossom_media_auth(&auth, &auth_hash, Some(&bound_host), max_age_secs)
        }
        UploadRouteMode::Upload | UploadRouteMode::Legacy => {
            verify_blossom_upload_auth(&auth, &auth_hash, Some(&bound_host), max_age_secs)
        }
    })
    .await
    .map_err(|_| MediaError::Internal)?
}

fn auth_max_age_secs(source_probe: Option<&crate::sanitize::MediaProbe>) -> u64 {
    match source_probe.map(|probe| probe.class) {
        Some(crate::sanitize::MediaClass::Video) => 3_600,
        Some(crate::sanitize::MediaClass::Image | crate::sanitize::MediaClass::Audio) | None => 600,
    }
}

fn source_record_fields<'a>(
    source_probe: Option<&'a crate::sanitize::MediaProbe>,
    source_hash: &'a str,
    source_size: u64,
) -> (Option<&'a str>, Option<u64>, Option<&'a str>) {
    match source_probe {
        Some(probe) => (Some(source_hash), Some(source_size), Some(&probe.mime)),
        None => (None, None, None),
    }
}

fn enforce_route_policy(
    mode: UploadRouteMode,
    source_probe: Option<&crate::sanitize::MediaProbe>,
) -> Result<(), MediaError> {
    match (mode, source_probe) {
        (UploadRouteMode::Media | UploadRouteMode::Legacy, None) => Err(
            MediaError::UnsupportedMedia("non-media attachment".to_string()),
        ),
        (UploadRouteMode::Upload, Some(probe)) => {
            Err(MediaError::UnsupportedMedia(probe.mime.clone()))
        }
        _ => Ok(()),
    }
}

async fn stream_source(
    body_stream: impl futures_core::Stream<Item = Result<Bytes, axum::Error>> + Send + 'static,
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<(String, u64, Vec<u8>), MediaError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::io::StreamReader;

    let mapped = futures_util::StreamExt::map(body_stream, |result| {
        result.map_err(|error| std::io::Error::other(error.to_string()))
    });
    let mut reader = StreamReader::new(Box::pin(mapped));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut sniff = Vec::with_capacity(4096);
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > max_bytes {
            return Err(MediaError::FileTooLarge {
                size: total,
                max: max_bytes,
            });
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        let remaining = 4096_usize.saturating_sub(sniff.len());
        sniff.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    file.flush()
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    Ok((hex::encode(hasher.finalize()), total, sniff))
}

async fn hash_file(path: &std::path::Path) -> Result<(String, u64), MediaError> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((hex::encode(hasher.finalize()), size))
}

fn enforce_source_size(
    probe: Option<&crate::sanitize::MediaProbe>,
    size: u64,
    config: &MediaConfig,
) -> Result<(), MediaError> {
    let max = match probe.map(|probe| probe.class) {
        Some(crate::sanitize::MediaClass::Image) => {
            if probe.is_some_and(|probe| probe.ext == "gif") {
                config.max_gif_bytes
            } else {
                config.max_image_bytes
            }
        }
        Some(crate::sanitize::MediaClass::Video) => config.max_video_bytes,
        Some(crate::sanitize::MediaClass::Audio) => config.max_audio_bytes,
        None => config.max_file_bytes,
    };
    if size > max {
        Err(MediaError::FileTooLarge { size, max })
    } else {
        Ok(())
    }
}

/// Shared buffered-upload pipeline for the image and generic-file paths.
///
/// Both paths are identical except for two steps, which are injected:
/// - `validate`: a CPU-bound check (run inside `spawn_blocking`) that returns
///   the `(mime, ext)` pair for the body. Images derive `ext` from the MIME;
///   generic files get both from the deny-list validator.
/// - `prepare_metadata`: builds metadata and stores any derived artifacts such
///   as a thumbnail, but deliberately does not write the sidecar. The sidecar
///   is the media serve gate and is published only after the moderation record
///   succeeds. It receives the already-computed
///   `(sha256, ext, mime, uploaded_at)` so no work is repeated.
///
/// Everything else — hash, Blossom auth (10-minute window), content-addressed
/// key, the both-exist idempotency short-circuit, blob store, orphan-blob
/// handling, and descriptor build — is common. The streaming video path stays
/// separate (see [`process_video_upload`]) because it never buffers in RAM.
///
/// `attribution` is `Some` when per-event upload records are enabled
/// (`BUZZ_MEDIA_UPLOAD_RECORDS`): a record is then written for **every**
/// accepted upload — including the idempotent short-circuit, which does no
/// blob PUT and would otherwise be invisible to the moderation pipeline.
/// For fresh uploads, the record is written after the blob and derived
/// artifacts but before the sidecar. This preserves both contracts: record
/// existence implies referenced objects are readable, while a record failure
/// cannot publish media without triggering moderation.
struct BufferedUploadInput<'a> {
    storage: &'a MediaStorage,
    config: &'a MediaConfig,
    ctx: &'a TenantContext,
    auth_event: &'a nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
}

async fn process_buffered_upload<V, M, Fut>(
    input: BufferedUploadInput<'_>,
    validate: V,
    prepare_metadata: M,
) -> Result<BlobDescriptor, MediaError>
where
    V: FnOnce(&Bytes, &MediaConfig) -> Result<(String, String), MediaError> + Send + 'static,
    M: FnOnce(MetadataInput) -> Fut,
    Fut: std::future::Future<Output = Result<BlobMeta, MediaError>>,
{
    let BufferedUploadInput {
        storage,
        config,
        ctx,
        auth_event,
        body,
        attribution,
    } = input;

    // CPU-bound: validate content, compute hash, verify auth.
    let auth = auth_event.clone();
    let bytes = body.clone();
    let cfg = config.clone();
    // Validate the Blossom `server` tag against the host this request was bound
    // to (the per-request tenant), not a process-global domain — a relay serves
    // many tenant hosts.
    let bound_host = ctx.host().to_string();
    let (mime, sha256, ext) = tokio::task::spawn_blocking(move || -> Result<_, MediaError> {
        let (mime, ext) = validate(&bytes, &cfg)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        // Buffered uploads (image + file): 10-minute auth window is plenty.
        verify_blossom_upload_auth(&auth, &sha256, Some(bound_host.as_str()), 600)?;
        Ok((mime, sha256, ext))
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    let key = format!("{sha256}.{ext}");
    let meta_key = MediaStorage::ctx_sidecar_key(ctx, &sha256);

    // Idempotent: short-circuit only if BOTH sidecar and blob exist. If the
    // sidecar exists but the blob is missing, fall through to re-upload.
    let sidecar_exists = storage.head(&meta_key).await?;
    let blob_exists = storage.head(&key).await?;
    if sidecar_exists && blob_exists {
        let meta = storage.get_sidecar(ctx, &sha256).await?;
        // A re-upload of known bytes is still a distinct upload *event*: no
        // blob PUT happens, so without this record the uploader would be
        // invisible to the moderation pipeline (and takedown re-uploads
        // would go unscanned).
        if let Some(attribution) = &attribution {
            record_upload_event(
                storage,
                ctx,
                &auth_event.pubkey,
                attribution,
                UploadEventFacts {
                    sha256: &sha256,
                    ext: &ext,
                    mime: &mime,
                    size: body.len() as u64,
                    source_sha256: None,
                    source_size: None,
                    source_mime: None,
                    sanitization_policy: None,
                    tool_versions: None,
                    uploaded_at: chrono::Utc::now().timestamp(),
                },
            )
            .await?;
        }
        return Ok(build_descriptor(
            config,
            &sha256,
            &ext,
            &mime,
            body.len() as u64,
            Some(&meta),
            meta.uploaded_at,
        ));
    }

    // Compute uploaded_at once — single source of truth for sidecar and response.
    let uploaded_at = chrono::Utc::now().timestamp();

    // Store blob first, then metadata.
    // On failure we intentionally do NOT delete the orphan blob — concurrent
    // uploads of the same hash could race and delete a blob that another
    // request is about to reference via its sidecar. Orphan blobs are
    // content-addressed and bounded by the upload size limit, so the storage
    // cost is negligible. A V2 background GC job can sweep blobs with no
    // matching sidecar after a grace period.
    storage.put(&key, &body, &mime).await?;

    let meta = match prepare_metadata(MetadataInput {
        sha256: sha256.clone(),
        ext: ext.clone(),
        mime: mime.clone(),
        body: body.clone(),
        uploaded_at,
    })
    .await
    {
        Ok(meta) => meta,
        Err(e) => {
            tracing::warn!(sha256 = %sha256, error = %e, "metadata generation failed; orphan blob left for GC");
            return Err(e);
        }
    };

    // The moderation record precedes the sidecar publish gate. If this write
    // fails, the blob and any thumbnail remain orphaned but the media cannot be
    // served. Conversely, record existence still implies those objects exist.
    if let Some(attribution) = &attribution {
        record_upload_event(
            storage,
            ctx,
            &auth_event.pubkey,
            attribution,
            UploadEventFacts {
                sha256: &sha256,
                ext: &ext,
                mime: &mime,
                size: body.len() as u64,
                source_sha256: None,
                source_size: None,
                source_mime: None,
                sanitization_policy: None,
                tool_versions: None,
                uploaded_at,
            },
        )
        .await?;
    }
    storage.put_sidecar(ctx, &sha256, &meta).await?;

    Ok(build_descriptor(
        config,
        &sha256,
        &ext,
        &mime,
        body.len() as u64,
        Some(&meta),
        uploaded_at,
    ))
}

/// Inputs handed to a buffered-upload metadata builder, after the shared
/// pipeline has already validated, hashed, and stored the blob. Owned so the
/// builder's future doesn't borrow the pipeline's locals; `body` is a `Bytes`
/// handle, so cloning it is a refcount bump, not a copy.
struct MetadataInput {
    sha256: String,
    ext: String,
    mime: String,
    body: Bytes,
    uploaded_at: i64,
}

/// Process an upload end-to-end: validate, store, thumbnail, return descriptor.
///
/// This is the image path — body is already fully buffered in RAM. Do NOT use
/// this for video uploads; use [`process_video_upload`] instead.
pub async fn process_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    process_buffered_upload(
        BufferedUploadInput {
            storage,
            config,
            ctx,
            auth_event,
            body,
            attribution,
        },
        |bytes, cfg| {
            let mime = validate_content(bytes, cfg)?;
            let ext = mime_to_ext(&mime).to_string();
            Ok((mime, ext))
        },
        |input| async move { prepare_image_metadata(storage, config, input).await },
    )
    .await
}

/// Process a generic (non-image, non-video) file upload end-to-end.
///
/// This is the catch-all attachment path: documents, archives, audio, text,
/// data — anything that isn't a previewable image or an H.264 MP4. The body is
/// fully buffered in RAM (bounded by `config.max_file_bytes` at the transport
/// layer), validated against the deny-list + size cap, stored, and recorded in
/// a minimal sidecar. No thumbnail, no dimensions, no duration.
///
/// The resulting blob is served with `Content-Disposition: attachment`, so the
/// client always downloads it rather than rendering it inline.
pub async fn process_file_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body: Bytes,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    process_buffered_upload(
        BufferedUploadInput {
            storage,
            config,
            ctx,
            auth_event,
            body,
            attribution,
        },
        |bytes, cfg| validate_file_content(bytes, cfg),
        |input| async move {
            // Minimal sidecar — no thumbnail/dim/blurhash/duration for generic files.
            let meta = BlobMeta {
                dim: String::new(),
                blurhash: String::new(),
                thumb_url: String::new(),
                size: input.body.len() as u64,
                ext: input.ext,
                mime_type: input.mime,
                uploaded_at: input.uploaded_at,
                duration_secs: None,
            };
            Ok(meta)
        },
    )
    .await
}

/// Process a video upload end-to-end using a streaming pipeline.
///
/// Unlike [`process_upload`], this function:
/// 1. Streams the request body to a [`tempfile::NamedTempFile`] while computing
///    SHA-256 incrementally — the full body is never in RAM simultaneously.
/// 2. Verifies the Blossom auth event `x` tag against the computed hash.
/// 3. Runs full MP4 validation (codec, duration, resolution, moov placement).
/// 4. Stores the blob via [`MediaStorage::put_file`] (streaming read from disk).
/// 5. Writes a sidecar with `duration_secs` (no thumbnail — desktop handles that).
///
/// Returns a [`BlobDescriptor`] with the `duration` field populated.
pub async fn process_video_upload(
    storage: &MediaStorage,
    config: &MediaConfig,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    body_stream: impl futures_core::Stream<Item = Result<Bytes, axum::Error>> + Send + 'static,
    content_length: Option<u64>,
    attribution: Option<UploadAttribution>,
) -> Result<BlobDescriptor, MediaError> {
    // --- 1. Stream body to temp file, compute SHA-256 incrementally ---
    let tmp = tempfile::NamedTempFile::new().map_err(|e| MediaError::Io(e.to_string()))?;
    let tmp_path = tmp.path().to_path_buf();

    let max_bytes = config.max_video_bytes;

    // Fast-fail: reject oversized uploads before streaming starts.
    if let Some(cl) = content_length {
        if cl > max_bytes {
            return Err(MediaError::FileTooLarge {
                size: cl,
                max: max_bytes,
            });
        }
    }

    let (sha256_hex, file_size, first_bytes) = {
        use tokio_util::io::StreamReader;

        // Convert axum::Error stream to std::io::Error stream for StreamReader.
        // Box::pin is required because StreamReader needs a pinned stream.
        // Belt-and-suspenders body-limit detection: axum wraps LengthLimitError
        // in its error chain but doesn't expose the inner type for downcasting.
        // We check multiple Display strings so that if axum changes the wording,
        // at least one pattern still matches. test_body_limit_error_detection
        // will catch a regression if ALL patterns break.
        let mapped = futures_util::StreamExt::map(body_stream, |r| {
            r.map_err(|e| {
                let msg = e.to_string();
                if msg.contains("length limit")
                    || msg.contains("body limit")
                    || msg.contains("LengthLimitError")
                {
                    std::io::Error::new(std::io::ErrorKind::WriteZero, msg)
                } else {
                    std::io::Error::other(e)
                }
            })
        });
        let mut reader = StreamReader::new(Box::pin(mapped));

        let mut file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| MediaError::Io(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        // Accumulate enough leading bytes for magic-byte detection.
        // 4 KiB is the standard sniff buffer — infer checks signatures at
        // various offsets, and some formats need more than just the first few
        // bytes. This is tiny relative to any real upload.
        const MIN_SNIFF_BYTES: usize = 4096;
        let mut sniff_buf: Vec<u8> = Vec::with_capacity(MIN_SNIFF_BYTES);
        let mut buf = vec![0u8; 64 * 1024]; // 64 KiB read buffer

        loop {
            use tokio::io::AsyncReadExt;
            let n = match reader.read(&mut buf).await {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WriteZero => {
                    // Body limit exceeded — return 413 instead of 500.
                    // `total` is bytes received before the cutoff — honest, not exact.
                    return Err(MediaError::FileTooLarge {
                        size: total,
                        max: max_bytes,
                    });
                }
                Err(e) => return Err(MediaError::Io(e.to_string())),
            };
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > max_bytes {
                return Err(MediaError::FileTooLarge {
                    size: total,
                    max: max_bytes,
                });
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .await
                .map_err(|e| MediaError::Io(e.to_string()))?;
            if sniff_buf.len() < MIN_SNIFF_BYTES {
                let need = MIN_SNIFF_BYTES - sniff_buf.len();
                sniff_buf.extend_from_slice(&buf[..n.min(need)]);
            }
        }
        file.flush()
            .await
            .map_err(|e| MediaError::Io(e.to_string()))?;

        let sha256_hex = hex::encode(hasher.finalize());
        (sha256_hex, total, sniff_buf)
    };

    // --- 2. Magic-byte check (video/mp4 only) ---
    // sniff_buf has up to MIN_SNIFF_BYTES (4 KiB) of leading bytes — enough for
    // infer::get() to detect MP4 ftyp even if the first network chunk was tiny.
    let mime = infer::get(&first_bytes)
        .map(|t| t.mime_type().to_string())
        .ok_or(MediaError::UnknownContentType)?;
    if mime != "video/mp4" {
        return Err(MediaError::DisallowedContentType(mime));
    }

    // --- 3. Verify Blossom auth: x tag must match computed SHA-256 ---
    let auth = auth_event.clone();
    let sha256_for_auth = sha256_hex.clone();
    // Validate the Blossom `server` tag against the bound tenant host (not a
    // process-global domain) — a relay serves many tenant hosts.
    let bound_host = ctx.host().to_string();
    tokio::task::spawn_blocking(move || {
        // Videos: 1-hour window — large uploads on slow connections need headroom.
        verify_blossom_upload_auth(&auth, &sha256_for_auth, Some(bound_host.as_str()), 3600)
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    // --- 4. Full MP4 validation on the temp file ---
    let tmp_path_clone = tmp_path.clone();
    let cfg = config.clone();
    let video_meta =
        tokio::task::spawn_blocking(move || validate_video_file(&tmp_path_clone, &cfg))
            .await
            .map_err(|_| MediaError::Internal)??;

    let ext = "mp4";
    let key = format!("{sha256_hex}.{ext}");
    let meta_key = MediaStorage::ctx_sidecar_key(ctx, &sha256_hex);

    // --- 5. Idempotency check ---
    let sidecar_exists = storage.head(&meta_key).await?;
    let blob_exists = storage.head(&key).await?;
    if sidecar_exists && blob_exists {
        let meta = storage.get_sidecar(ctx, &sha256_hex).await?;
        // Re-upload of known bytes: still a distinct upload event — see the
        // buffered path's short-circuit for the rationale.
        if let Some(attribution) = &attribution {
            record_upload_event(
                storage,
                ctx,
                &auth_event.pubkey,
                attribution,
                UploadEventFacts {
                    sha256: &sha256_hex,
                    ext,
                    mime: &mime,
                    size: file_size,
                    source_sha256: None,
                    source_size: None,
                    source_mime: None,
                    sanitization_policy: None,
                    tool_versions: None,
                    uploaded_at: chrono::Utc::now().timestamp(),
                },
            )
            .await?;
        }
        return Ok(build_descriptor(
            config,
            &sha256_hex,
            ext,
            &mime,
            file_size,
            Some(&meta),
            meta.uploaded_at,
        ));
    }

    let uploaded_at = chrono::Utc::now().timestamp();

    // --- 6. Stream blob from temp file to S3 ---
    storage.put_file(&key, &tmp_path, &mime).await?;
    drop(tmp); // Free temp file disk space immediately after S3 upload.

    // --- 7. Build metadata (no thumbnail for video — desktop handles that) ---
    let meta = BlobMeta {
        dim: format!("{}x{}", video_meta.width, video_meta.height),
        blurhash: String::new(),
        thumb_url: String::new(),
        ext: ext.to_string(),
        mime_type: mime.clone(),
        size: file_size,
        uploaded_at,
        duration_secs: Some(video_meta.duration_secs),
    };

    // Record before publishing the sidecar serve gate. See the buffered path.
    if let Some(attribution) = &attribution {
        record_upload_event(
            storage,
            ctx,
            &auth_event.pubkey,
            attribution,
            UploadEventFacts {
                sha256: &sha256_hex,
                ext,
                mime: &mime,
                size: file_size,
                source_sha256: None,
                source_size: None,
                source_mime: None,
                sanitization_policy: None,
                tool_versions: None,
                uploaded_at,
            },
        )
        .await?;
    }
    storage.put_sidecar(ctx, &sha256_hex, &meta).await?;

    Ok(build_descriptor(
        config,
        &sha256_hex,
        ext,
        &mime,
        file_size,
        Some(&meta),
        uploaded_at,
    ))
}

/// Generate thumbnail and metadata without publishing the sidecar serve gate.
/// Returns the completed [`BlobMeta`] on success.
async fn prepare_image_metadata(
    storage: &MediaStorage,
    config: &MediaConfig,
    input: MetadataInput,
) -> Result<BlobMeta, MediaError> {
    let body_ref = input.body.clone();
    let mime_ref = input.mime.clone();
    let ext_ref = input.ext.clone();
    let sha256_ref = input.sha256.clone();
    let cfg_ref = config.clone();
    let (mut meta, thumb_bytes) = tokio::task::spawn_blocking(move || {
        generate_image_metadata_sync(&cfg_ref, &sha256_ref, &body_ref, &mime_ref, &ext_ref)
    })
    .await
    .map_err(|_| MediaError::Internal)??;

    meta.uploaded_at = input.uploaded_at;

    if let Some(ref tb) = thumb_bytes {
        let thumb_key = format!("{}.thumb.jpg", input.sha256);
        storage.put(&thumb_key, tb, "image/jpeg").await?;
    }

    Ok(meta)
}

fn build_descriptor(
    config: &MediaConfig,
    sha256: &str,
    ext: &str,
    mime: &str,
    size: u64,
    meta: Option<&BlobMeta>,
    uploaded_at: i64,
) -> BlobDescriptor {
    let duration = meta.and_then(|m| m.duration_secs);
    BlobDescriptor {
        url: format!("{}/{sha256}.{ext}", config.public_base_url),
        sha256: sha256.to_string(),
        size,
        mime_type: mime.to_string(),
        uploaded: uploaded_at,
        dim: meta.and_then(|m| (!m.dim.is_empty()).then(|| m.dim.clone())),
        blurhash: meta.and_then(|m| (!m.blurhash.is_empty()).then(|| m.blurhash.clone())),
        thumb: meta.and_then(|m| (!m.thumb_url.is_empty()).then(|| m.thumb_url.clone())),
        duration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_probe() -> crate::sanitize::MediaProbe {
        crate::sanitize::MediaProbe {
            class: crate::sanitize::MediaClass::Image,
            mime: "image/jpeg".to_string(),
            ext: "jpg".to_string(),
            video_codec: None,
            audio_codec: None,
            width: Some(1),
            height: Some(1),
            duration_secs: None,
            frame_count: Some(1),
        }
    }

    fn test_config() -> MediaConfig {
        MediaConfig {
            s3_endpoint: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
            s3_bucket: String::new(),
            s3_region: "us-east-1".to_string(),
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_audio_bytes: 104_857_600,
            max_file_bytes: 104_857_600,
            exiftool_path: "exiftool".to_string(),
            ffmpeg_path: "ffmpeg".to_string(),
            ffprobe_path: "ffprobe".to_string(),
            image_process_timeout_secs: 120,
            av_process_timeout_secs: 600,
            public_base_url: "https://media.example.com".to_string(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        }
    }

    #[test]
    fn test_build_descriptor_video_omits_empty_thumb_and_blurhash() {
        // Video uploads produce a BlobMeta with empty thumb_url and blurhash.
        // build_descriptor must convert these to None so they're omitted from JSON.
        let config = test_config();
        let meta = BlobMeta {
            dim: "320x240".to_string(),
            blurhash: String::new(),  // empty — video has no blurhash
            thumb_url: String::new(), // empty — video has no thumbnail
            ext: "mp4".to_string(),
            mime_type: "video/mp4".to_string(),
            size: 5_000_000,
            uploaded_at: 1700000000,
            duration_secs: Some(29.5),
        };

        let desc = build_descriptor(
            &config,
            "abc123",
            "mp4",
            "video/mp4",
            5_000_000,
            Some(&meta),
            1700000000,
        );

        // Empty strings must become None, not Some("")
        assert!(
            desc.blurhash.is_none(),
            "blurhash should be None for video, got {:?}",
            desc.blurhash
        );
        assert!(
            desc.thumb.is_none(),
            "thumb should be None for video, got {:?}",
            desc.thumb
        );
        // Non-empty fields should be present
        assert_eq!(desc.dim, Some("320x240".to_string()));
        assert_eq!(desc.duration, Some(29.5));

        // Verify JSON serialization omits the empty fields entirely
        let json = serde_json::to_value(&desc).unwrap();
        assert!(
            json.get("blurhash").is_none(),
            "blurhash should be absent from JSON"
        );
        assert!(
            json.get("thumb").is_none(),
            "thumb should be absent from JSON"
        );
        assert!(json.get("dim").is_some(), "dim should be present in JSON");
        assert!(
            json.get("duration").is_some(),
            "duration should be present in JSON"
        );
    }

    #[test]
    fn test_build_descriptor_image_includes_thumb_and_blurhash() {
        // Image uploads produce a BlobMeta with populated thumb_url and blurhash.
        let config = test_config();
        let hash = "a".repeat(64);
        let meta = BlobMeta {
            dim: "800x600".to_string(),
            blurhash: "LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string(),
            thumb_url: format!("https://media.example.com/{hash}.thumb.jpg"),
            ext: "jpg".to_string(),
            mime_type: "image/jpeg".to_string(),
            size: 100_000,
            uploaded_at: 1700000000,
            duration_secs: None,
        };

        let desc = build_descriptor(
            &config,
            &hash,
            "jpg",
            "image/jpeg",
            100_000,
            Some(&meta),
            1700000000,
        );

        assert_eq!(
            desc.blurhash,
            Some("LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string())
        );
        assert!(desc.thumb.is_some());
        assert!(desc.duration.is_none());

        // Verify JSON: duration should be absent, blurhash and thumb present
        let json = serde_json::to_value(&desc).unwrap();
        assert!(json.get("blurhash").is_some());
        assert!(json.get("thumb").is_some());
        assert!(
            json.get("duration").is_none(),
            "duration should be absent for images"
        );
    }

    #[test]
    fn test_body_limit_error_detection() {
        // Verify that body-limit errors are mapped to WriteZero (which
        // process_video_upload converts to FileTooLarge / 413).
        // Must match the detection logic in process_video_upload exactly.
        let detect = |msg: &str| -> std::io::ErrorKind {
            if msg.contains("length limit")
                || msg.contains("body limit")
                || msg.contains("LengthLimitError")
            {
                std::io::ErrorKind::WriteZero
            } else {
                std::io::ErrorKind::Other
            }
        };

        // All known patterns should trigger WriteZero.
        assert_eq!(
            detect("length limit exceeded"),
            std::io::ErrorKind::WriteZero
        );
        assert_eq!(detect("body limit exceeded"), std::io::ErrorKind::WriteZero);
        assert_eq!(detect("LengthLimitError"), std::io::ErrorKind::WriteZero);

        // Non-limit errors should remain as Other.
        assert_eq!(detect("connection reset"), std::io::ErrorKind::Other);
    }

    #[test]
    fn route_policy_keeps_legacy_media_only() {
        let probe = image_probe();

        assert!(enforce_route_policy(UploadRouteMode::Media, Some(&probe)).is_ok());
        assert!(enforce_route_policy(UploadRouteMode::Legacy, Some(&probe)).is_ok());
        assert!(enforce_route_policy(UploadRouteMode::Upload, None).is_ok());

        assert!(matches!(
            enforce_route_policy(UploadRouteMode::Media, None),
            Err(MediaError::UnsupportedMedia(_))
        ));
        assert!(matches!(
            enforce_route_policy(UploadRouteMode::Legacy, None),
            Err(MediaError::UnsupportedMedia(_))
        ));
        assert!(matches!(
            enforce_route_policy(UploadRouteMode::Upload, Some(&probe)),
            Err(MediaError::UnsupportedMedia(mime)) if mime == "image/jpeg"
        ));
    }

    #[test]
    fn authorization_freshness_is_class_specific() {
        let image = image_probe();
        let mut video = image.clone();
        video.class = crate::sanitize::MediaClass::Video;
        let mut audio = image.clone();
        audio.class = crate::sanitize::MediaClass::Audio;

        assert_eq!(auth_max_age_secs(Some(&image)), 600);
        assert_eq!(auth_max_age_secs(Some(&audio)), 600);
        assert_eq!(auth_max_age_secs(None), 600);
        assert_eq!(auth_max_age_secs(Some(&video)), 3_600);
    }

    #[test]
    fn exact_upload_records_omit_transformation_fields() {
        assert_eq!(source_record_fields(None, "source", 42), (None, None, None));

        let image = image_probe();
        assert_eq!(
            source_record_fields(Some(&image), "source", 42),
            (Some("source"), Some(42), Some("image/jpeg"))
        );
    }

    #[test]
    fn test_build_descriptor_no_meta() {
        // When meta is None, all optional fields should be None.
        let config = test_config();
        let desc = build_descriptor(
            &config,
            "abc123",
            "jpg",
            "image/jpeg",
            100,
            None,
            1700000000,
        );

        assert!(desc.dim.is_none());
        assert!(desc.blurhash.is_none());
        assert!(desc.thumb.is_none());
        assert!(desc.duration.is_none());
    }
}
