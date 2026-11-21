#!/usr/bin/env bash
# Download only pinned upstream release bytes, never HTTP-success challenge pages.
set -euo pipefail

fetch_verified() {
  local destination="$1" digest="$2" url temporary
  shift 2
  mkdir -p "$(dirname "$destination")"
  if [ -f "$destination" ] && printf '%s  %s\n' "$digest" "$destination" | shasum -a 256 -c - >/dev/null; then
    return 0
  fi
  for url in "$@"; do
    temporary="$(mktemp "$destination.partial.XXXXXX")"
    if curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
      --connect-timeout 15 --max-time 120 --max-filesize 67108864 --retry 2 --retry-delay 2 \
      --output "$temporary" "$url" && \
      printf '%s  %s\n' "$digest" "$temporary" | shasum -a 256 -c - >/dev/null; then
      mv -f "$temporary" "$destination"
      printf 'Verified source: %s\n' "$destination"
      return 0
    fi
    rm -f "$temporary"
    printf 'Rejected download or checksum mismatch: %s\n' "$url" >&2
  done
  printf 'No verified upstream source available for %s\n' "$destination" >&2
  return 1
}

main() {
  local output="${1:?Usage: fetch-media-sources.sh output-directory}"
  fetch_verified "$output/ffmpeg-8.0.1.tar.xz" \
    05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41 \
    https://ffmpeg.org/releases/ffmpeg-8.0.1.tar.xz \
    https://www.ffmpeg.org/releases/ffmpeg-8.0.1.tar.xz
  fetch_verified "$output/lame-3.100.tar.gz" \
    ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e \
    https://downloads.sourceforge.net/project/lame/lame/3.100/lame-3.100.tar.gz \
    https://sourceforge.net/projects/lame/files/lame/3.100/lame-3.100.tar.gz/download
  # These two official release assets have the same pinned digest. zlib.net can
  # return an HTML challenge with HTTP 200 on hosted runners; never accept it.
  fetch_verified "$output/zlib-1.3.1.tar.gz" \
    9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23 \
    https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz \
    https://zlib.net/fossils/zlib-1.3.1.tar.gz

}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
