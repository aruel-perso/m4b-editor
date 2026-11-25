#!/usr/bin/env bash
# Native Unix build with pinned static FFmpeg, LAME and zlib.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
source "$root/scripts/ffmpeg-options.sh"
platform="${1:?Pass macos or linux}"
shift
case "$platform" in
  macos)
    [ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] || exit 1
    export MACOSX_DEPLOYMENT_TARGET=11.0 CC=/usr/bin/clang
    export CFLAGS='-arch arm64 -mmacosx-version-min=11.0'
    target=(--arch=aarch64 --target-os=darwin)
    ;;
  linux)
    [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || exit 1
    export CC=/usr/bin/gcc
    target=(--arch=x86_64 --target-os=linux)
    ;;
  *) exit 1 ;;
esac
source_archive="${1:?Pass ffmpeg-8.0.1.tar.xz}"
zlib_archive="${2:?Pass zlib-1.3.1.tar.gz}"
source_archive="$(cd "$(dirname "$source_archive")" && pwd)/$(basename "$source_archive")"
zlib_archive="$(cd "$(dirname "$zlib_archive")" && pwd)/$(basename "$zlib_archive")"
printf '%s  %s\n' 05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41 "$source_archive" | shasum -a 256 -c -
printf '%s  %s\n' 9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23 "$zlib_archive" | shasum -a 256 -c -
lame_archive="${3:?Pass pinned lame-3.100.tar.gz}"
lame_archive="$(cd "$(dirname "$lame_archive")" && pwd)/$(basename "$lame_archive")"
printf '%s  %s\n' ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e "$lame_archive" | shasum -a 256 -c -
out="$root/vendor/$platform-media"
mkdir -p "$out" "${TMPDIR:?Set TMPDIR to writable scratch}"
build="$(mktemp -d "$TMPDIR/m4b-media.XXXXXX")"
trap 'rm -rf "$build"' EXIT
tar -xf "$source_archive" -C "$build"
tar -xf "$zlib_archive" -C "$build"
cd "$build/zlib-1.3.1"
./configure --static --prefix="$build/static"
make -j1
make install
# Record zlib's generated configuration; avoid retaining intermediate libraries.
cat zconf.h Makefile > "$out/zlib-config.txt"
tar -xf "$lame_archive" -C "$build"
cd "$build/lame-3.100"
./configure --disable-shared --enable-static --disable-frontend --prefix="$build/static"
make -j1
make install
cp COPYING "$out/COPYING.LAME"
cp config.log "$out/lame-config.txt"
cp "$lame_archive" "$out/lame-3.100.tar.gz"
cd "$build/ffmpeg-8.0.1"
./configure --pkg-config=/usr/bin/false --prefix="$out" --cc="$CC" "${target[@]}" \
  --disable-autodetect --disable-x86asm "${FFMPEG_OPTIONS[@]}" \
  --enable-zlib --extra-cflags="${CFLAGS:-} -I$build/static/include" \
  --extra-ldflags="${CFLAGS:-} -L$build/static/lib"
make -j1 ffmpeg ffprobe
cp ffmpeg ffprobe config.h config_components.h ffbuild/config.mak COPYING.LGPLv2.1 LICENSE.md "$out/"
cp "$build/zlib-1.3.1/LICENSE" "$out/LICENSE.zlib"
cp "$source_archive" "$out/ffmpeg-8.0.1.tar.xz"
cp "$zlib_archive" "$out/zlib-1.3.1.tar.gz"
(cd "$out" && shasum -a 256 ffmpeg ffprobe ffmpeg-8.0.1.tar.xz zlib-1.3.1.tar.gz config.h config_components.h config.mak COPYING.LGPLv2.1 LICENSE.md LICENSE.zlib zlib-config.txt lame-3.100.tar.gz COPYING.LAME lame-config.txt > SHA256SUMS)
if [ "$platform" = macos ]; then
  python3 "$root/scripts/verify-macos.py" "$out/ffmpeg" "$out/ffprobe"
fi
