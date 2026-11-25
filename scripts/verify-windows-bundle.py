#!/usr/bin/env python3
"""Execute Windows bundle end-to-end under Wine, without installing media tools.
Usage: python scripts/verify-windows-bundle.py dist/M4B-Editor-...zip
Wine is a developer-only test requirement, not an end-user dependency.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import wave
import zipfile
import zlib
from release_common import ROOT, VERSION, verify_source_archives


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('archive', type=Path)
    args = parser.parse_args()
    scratch = Path(os.environ.get('TMPDIR', str(Path.home() / '.hermes/cache/scratch')))
    scratch.mkdir(parents=True, exist_ok=True)
    wine = '/usr/lib/wine/wine64'
    env = dict(os.environ, WINEDEBUG='-all', WINEPATH='',
               WINEPREFIX=str(scratch / 'm4b-bundle-verification-wine'))
    with tempfile.TemporaryDirectory(prefix='m4b-bundle-verify-', dir=scratch) as temp:
        temp = Path(temp)
        with zipfile.ZipFile(args.archive) as archive:
            assert archive.testzip() is None
            archive.extractall(temp / 'extracted')
        package = temp/'extracted'
        if not (package/'m4b-editor-cli.exe').is_file():
            package = next(package.iterdir())
        cwd = temp / 'unrelated directory été'
        cwd.mkdir()
        # Poison CWD: runtime must locate its packaged tools by executable directory.
        for name in ['ffmpeg.exe', 'ffprobe.exe']:
            (cwd / name).write_bytes(b'not an executable')
        def win(path):
            return 'Z:' + str(path.resolve()).replace('/', '\\')
        def run(executable, *arguments, ok=True):
            command = [wine, str(executable), *map(str, arguments)]
            result = subprocess.run(command, cwd=cwd, env=env, capture_output=True,
                                    text=True, timeout=90)
            if ok and result.returncode:
                raise AssertionError(f'{command}\n{result.stdout}\n{result.stderr}')
            if not ok:
                assert result.returncode != 0
            return result
        cli = package / 'm4b-editor-cli.exe'
        assert VERSION in run(cli, '--version').stdout
        assert 'aac' in run(cli, 'encoders').stdout.splitlines()
        tracks = []
        for number, rate, frequency in [(2, 44100, 440), (10, 32000, 880)]:
            path = cwd / f'piste {number} été.wav'
            with wave.open(str(path), 'wb') as wav:
                wav.setnchannels(1); wav.setsampwidth(2); wav.setframerate(rate)
                wav.writeframes(b''.join(struct.pack('<h', int(10000 * math.sin(2 * math.pi * frequency * i / rate))) for i in range(rate)))
            tracks.append(path)
        for codec, extension in [('libmp3lame','mp3'),('flac','flac')]:
            encoded = cwd/f'codec-smoke.{extension}'
            run(package/'ffmpeg.exe','-v','error','-nostdin','-i',win(tracks[0]),'-c:a',codec,win(encoded))
            run(package/'ffmpeg.exe','-v','error','-nostdin','-i',win(encoded),'-f','null','-')
            data = json.loads(run(package/'ffprobe.exe','-v','error','-show_streams','-of','json',win(encoded)).stdout)
            assert data['streams'][0]['codec_name'] == extension
        def png_chunk(kind, data):
            return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
        cover = cwd / 'couverture.png'
        cover.write_bytes(b'\x89PNG\r\n\x1a\n' + png_chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 2, 0, 0, 0)) + png_chunk(b'IDAT', zlib.compress((b'\0' + b'\xff\0\0' * 2) * 2)) + png_chunk(b'IEND', b''))
        project, output = cwd / 'livre.json', cwd / 'livre.m4b'
        hashes = [hashlib.sha256(p.read_bytes()).digest() for p in tracks]
        run(cli, 'new', '--project', win(project), '--title', 'Livre été', '--author', 'Auteur', '--cover', win(cover), win(tracks[1]), win(tracks[0]))
        run(cli, 'export', win(project), win(output))
        data = json.loads(run(cli, 'probe', win(output)).stdout)
        assert data['metadata']['title'] == 'Livre été'
        assert len(data['chapters']) == 2 and data['cover'] is not None
        assert data['tracks'][0]['codec'] == 'aac'
        original = output.read_bytes()
        run(cli, 'export', win(project), win(output), ok=False)
        assert output.read_bytes() == original
        assert hashes == [hashlib.sha256(p.read_bytes()).digest() for p in tracks]
        # Exercise the exact preview decoder command on the bundled M4B and backend.
        pcm = subprocess.run([wine, str(package / 'ffmpeg.exe'), '-hide_banner', '-loglevel', 'error', '-nostdin', '-ss', '0.500', '-i', win(output), '-map', '0:a:0', '-vn', '-f', 's16le', '-acodec', 'pcm_s16le', '-ar', '48000', '-ac', '2', 'pipe:1'], cwd=cwd, env=env, capture_output=True, timeout=60)
        assert pcm.returncode == 0, pcm.stderr.decode(errors='replace')
        assert len(pcm.stdout) > 96000 and any(pcm.stdout)
        print('PASS bundled Windows tools: natural-order WAV import -> AAC M4B + PNG cover + chapters -> probe -> non-silent preview PCM at seek position')
        print('PASS unrelated Unicode CWD with poisoned FFmpeg names; no WINEPATH media tools; originals and existing output preserved')
        assert not any((package/name).exists() for name in ['7z.exe', '7z.dll'])
        for fixture in ['audio.zip', 'audio.rar']:
            imported = temp/(fixture + '.m4bproj')
            run(cli, 'new', '--project', win(imported), win(ROOT/'tests/fixtures'/fixture))
            assert len(json.loads(imported.read_text())['tracks']) == 1
        # Actual application binary sources and media sources must be included.
        assert (package / 'source/crates/desktop/src/playback.rs').is_file()
        verify_source_archives(package/'source/third-party/ffmpeg')
        print('PASS corresponding application source and embedded media source present')
    subprocess.run(['/usr/lib/wine/wineserver64', '-k'], env=env, timeout=20, check=False)


if __name__ == '__main__':
    main()
