#!/usr/bin/env python3
"""Exercise the installed and portable Linux payloads after CI installs the DEB."""
import argparse
import os
import subprocess
import tempfile
from pathlib import Path
from release_common import ROOT, VERSION, verify_source_archives


def exercise(payload, cli, work):
    for name in ('m4b-editor', 'm4b-editor-cli', 'ffmpeg', 'ffprobe'):
        assert os.access(payload/name, os.X_OK), name
    assert (payload/'THIRD-PARTY-NOTICES.txt').is_file()
    assert (payload/'source/Cargo.lock').is_file()
    verify_source_archives(payload/'source/third-party/ffmpeg')
    env = dict(os.environ, PATH=str(work/'no-path-tools'))

    def run(*args, **kwargs):
        return subprocess.check_output([str(cli), *map(str, args)], env=env, **kwargs)

    assert VERSION in run('--version', text=True)
    project = work/'fixture.m4bproj'
    book = work/'fixture.m4b'
    run('new', '--project', project, ROOT/'tests/fixtures/audio.zip')
    run('check', project)
    run('export', project, book)
    run('probe', book)
    assert book.stat().st_size > 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--deb', required=True, type=Path)
    parser.add_argument('--scratch', required=True, type=Path)
    args = parser.parse_args()
    info = subprocess.check_output(['dpkg-deb', '-f', args.deb, 'Architecture'], text=True)
    assert info.strip() == 'amd64'
    assert Path('/usr/share/applications/m4b-editor.desktop').is_file()
    assert Path('/usr/share/pixmaps/m4b-editor.png').is_file()
    with tempfile.TemporaryDirectory(prefix='m4b-linux-test-', dir=args.scratch) as temporary:
        work = Path(temporary)
        installed = work/'installed'
        installed.mkdir()
        exercise(Path('/usr/lib/m4b-editor'), Path('/usr/bin/m4b-editor-cli'), installed)
        portable = work/'portable'
        subprocess.run(['unzip', '-q', args.deb.with_suffix('.zip'), '-d', portable], check=True)
        exercise(portable, portable/'m4b-editor-cli', portable)
    print('PASS Linux installed and portable archive import, adjacent media tools and M4B export')


if __name__ == '__main__':
    main()
