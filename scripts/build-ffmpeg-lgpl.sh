#!/bin/sh

# Build the relay's FFmpeg/ffprobe toolchain without GPL or non-redistributable
# components. The only external codec enabled here is BSD-licensed OpenH264;
# all other media support comes from FFmpeg's LGPL implementation.

set -eu

: "${FFMPEG_VERSION:=7.1.5}"
: "${FFMPEG_SHA256:=de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f}"
: "${FFMPEG_PREFIX:=/opt/ffmpeg}"

archive="/tmp/ffmpeg-${FFMPEG_VERSION}.tar.xz"
source_dir="/tmp/ffmpeg-${FFMPEG_VERSION}"

curl --fail --location --silent --show-error \
  "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz" \
  --output "${archive}"
echo "${FFMPEG_SHA256}  ${archive}" | sha256sum --check --strict
tar --extract --file "${archive}" --directory /tmp

cd "${source_dir}"
./configure \
  --prefix="${FFMPEG_PREFIX}" \
  --disable-autodetect \
  --disable-debug \
  --disable-doc \
  --disable-ffplay \
  --disable-network \
  --disable-static \
  --enable-libopenh264 \
  --enable-shared \
  --extra-ldflags="-Wl,-rpath,${FFMPEG_PREFIX}/lib" \
  --enable-zlib

make --jobs="$(getconf _NPROCESSORS_ONLN)"
make install

license_dir="${FFMPEG_PREFIX}/share/licenses/ffmpeg"
openh264_license_dir="${FFMPEG_PREFIX}/share/licenses/openh264"
source_archive_dir="${FFMPEG_PREFIX}/share/source/ffmpeg"
mkdir -p "${license_dir}" "${openh264_license_dir}" "${source_archive_dir}"
cp COPYING.LGPLv2.1 LICENSE.md "${license_dir}/"
cp /usr/share/doc/libopenh264-dev/copyright "${openh264_license_dir}/"
cp "${archive}" "${source_archive_dir}/"

"${FFMPEG_PREFIX}/bin/ffmpeg" -hide_banner -buildconf \
  > "${source_archive_dir}/build-configuration.txt" 2>&1

if grep -q -- '--enable-gpl\|--enable-nonfree' \
  "${source_archive_dir}/build-configuration.txt"; then
  echo "Refusing to package a GPL or non-redistributable FFmpeg build" >&2
  exit 1
fi

"${FFMPEG_PREFIX}/bin/ffmpeg" -hide_banner -encoders 2>&1 \
  | grep -q 'libopenh264'
"${FFMPEG_PREFIX}/bin/ffprobe" -hide_banner -version >/dev/null
