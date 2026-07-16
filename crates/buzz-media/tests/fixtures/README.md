# Media compliance fixtures

The compliance test builds deterministic, synthetic fixtures at runtime so
codec/container coverage follows the configured FFmpeg and ExifTool binaries.
The published relay image builds its FFmpeg/ffprobe executables from the pinned
source and configuration in `scripts/build-ffmpeg-lgpl.sh`. Coordinates are
fictional test data (`41.8781, -87.6298`) and no fixture contains a real person
or device identifier.

`tiny.heic.b64` is the sole prebuilt input because FFmpeg does not provide a
portable HEIC muxer. It is a 64×64 synthetic application icon converted by
macOS ImageIO. The test adds GPS, camera, timestamp, and comment metadata with
ExifTool before passing it through the same sanitizer used by the relay.

The matrix covers JPEG, PNG, GIF, WebP, HEIC, AVIF, TIFF, BMP, MP4, MOV, WebM,
MKV, MP3, M4A, AAC, FLAC, WAV, Ogg/Vorbis, and Opus. Every generated source is
small (96×64 or one second) and deterministic.
