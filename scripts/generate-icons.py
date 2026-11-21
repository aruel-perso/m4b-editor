#!/usr/bin/env python3
"""Regenerate app icons from SVG using FFmpeg with the librsvg decoder."""
import argparse
from pathlib import Path
import struct
import subprocess

ICONS = Path(__file__).resolve().parents[1] / 'packaging/icons'
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
ICNS_SIZES = {
    b'icp4': 16, b'icp5': 32, b'icp6': 64, b'ic07': 128, b'ic08': 256,
    b'ic09': 512, b'ic10': 1024, b'ic11': 32, b'ic12': 64,
    b'ic13': 256, b'ic14': 512,
}


def render(size):
    return subprocess.check_output([
        'ffmpeg', '-v', 'error', '-nostdin', '-c:v', 'librsvg',
        '-width', str(size), '-height', str(size), '-i', str(ICONS / 'app.svg'),
        '-frames:v', '1', '-c:v', 'png', '-pix_fmt', 'rgba', '-threads', '1',
        '-fflags', '+bitexact', '-flags:v', '+bitexact', '-f', 'image2pipe', '-',
    ])


def ico(images):
    offset = 6 + 16 * len(ICO_SIZES)
    directory = bytearray(struct.pack('<HHH', 0, 1, len(ICO_SIZES)))
    for size in ICO_SIZES:
        length = len(images[size])
        directory.extend(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0,
                                     1, 32, length, offset))
        offset += length
    return bytes(directory) + b''.join(images[size] for size in ICO_SIZES)


def icns(images):
    chunks = b''.join(struct.pack('>4sI', kind, 8 + len(images[size])) + images[size]
                      for kind, size in ICNS_SIZES.items())
    return struct.pack('>4sI', b'icns', 8 + len(chunks)) + chunks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Compare with the checked-in assets')
    args = parser.parse_args()
    images = {size: render(size) for size in sorted(set(ICO_SIZES) | set(ICNS_SIZES.values()))}
    for name, data in [('app.png', images[64]), ('app.ico', ico(images)), ('app.icns', icns(images))]:
        path = ICONS / name
        if args.check:
            if not path.is_file() or path.read_bytes() != data:
                raise SystemExit(f'{path} differs from generated output')
        else:
            path.write_bytes(data)


if __name__ == '__main__':
    main()
