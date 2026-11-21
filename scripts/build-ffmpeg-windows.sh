#!/usr/bin/env bash
# Build a small self-contained Windows media backend from unmodified FFmpeg source.
# Developer prerequisites: make, x86_64-w64-mingw32-gcc, tar, xz, POSIX shell.
# End users need none of these: package ffmpeg.exe/ffprobe.exe beside the app.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
source_archive="${1:?Usage: build-ffmpeg-windows.sh ffmpeg-8.0.1.tar.xz zlib-1.3.1.tar.gz}"
zlib_archive="${2:?Pass the zlib-1.3.1.tar.gz source archive}"
# Verify pinned source before extraction or execution of configure/make.
printf '%s  %s\n' '05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41' "$source_archive" | sha256sum -c -
printf '%s  %s\n' '9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23' "$zlib_archive" | sha256sum -c -
source "$root/scripts/ffmpeg-options.sh"
source_archive="$(cd "$(dirname "$source_archive")" && pwd)/$(basename "$source_archive")"
zlib_archive="$(cd "$(dirname "$zlib_archive")" && pwd)/$(basename "$zlib_archive")"
lame_archive="${3:?Pass pinned lame-3.100.tar.gz}"
lame_archive="$(cd "$(dirname "$lame_archive")" && pwd)/$(basename "$lame_archive")"
printf '%s  %s\n' ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e "$lame_archive" | shasum -a 256 -c -
out="$root/vendor/windows-media"
mkdir -p "$out"
build="${TMPDIR:-$HOME/.hermes/cache/scratch}/m4b-ffmpeg-build"
mkdir -p "$build"
tar -xf "$source_archive" -C "$build"
tar -xf "$zlib_archive" -C "$build"
cd "$build/zlib-1.3.1"
make -j1 -f win32/Makefile.gcc PREFIX=x86_64-w64-mingw32- libz.a
zlib="$build/zlib-1.3.1"
tar -xf "$lame_archive" -C "$build"
cd "$build/lame-3.100"
CC=x86_64-w64-mingw32-gcc AR=x86_64-w64-mingw32-ar RANLIB=x86_64-w64-mingw32-ranlib ./configure --host=x86_64-w64-mingw32 --disable-shared --enable-static --disable-frontend --prefix="$build/static"
make -j1
make install
cp COPYING "$out/COPYING.LAME"
cp config.log "$out/lame-config.txt"
cp "$lame_archive" "$out/lame-3.100.tar.gz"
cd "$build/ffmpeg-8.0.1"
./configure --pkg-config=/usr/bin/false \
  --prefix="$out" --enable-cross-compile --target-os=mingw32 --arch=x86_64 \
  --cross-prefix=x86_64-w64-mingw32- --disable-x86asm --disable-autodetect \
  "${FFMPEG_OPTIONS[@]}" \
  --enable-zlib --extra-cflags="-I$zlib -I$build/static/include" --extra-ldflags="-static -L$zlib -L$build/static/lib"
make -j1 ffmpeg.exe ffprobe.exe
cp ffmpeg.exe ffprobe.exe "$out/"
cp config.h config_components.h ffbuild/config.mak "$out/"
cp COPYING.LGPLv2.1 LICENSE.md "$out/"
if [ "$(readlink -f "$source_archive")" != "$(readlink -f "$out/ffmpeg-8.0.1.tar.xz")" ]; then cp "$source_archive" "$out/ffmpeg-8.0.1.tar.xz"; fi
if [ "$(readlink -f "$zlib_archive")" != "$(readlink -f "$out/zlib-1.3.1.tar.gz")" ]; then cp "$zlib_archive" "$out/zlib-1.3.1.tar.gz"; fi
cp "$zlib/LICENSE" "$out/LICENSE.zlib"
printf '%s\n' 'make -j1 -f win32/Makefile.gcc PREFIX=x86_64-w64-mingw32- libz.a' > "$out/zlib-config.txt"
(cd "$out" && sha256sum ffmpeg.exe ffprobe.exe ffmpeg-8.0.1.tar.xz zlib-1.3.1.tar.gz config.h config_components.h config.mak COPYING.LGPLv2.1 LICENSE.md LICENSE.zlib zlib-config.txt lame-3.100.tar.gz COPYING.LAME lame-config.txt > SHA256SUMS)
printf 'Built bundled Windows media tools in %s\n' "$out"
