#!/usr/bin/env python3
"""Build Linux x86_64 DEB and portable ZIP with adjacent pinned media tools."""
import argparse
import os
import platform
import shutil
import subprocess
import tempfile
from pathlib import Path
from release_common import (ROOT, VERSION, archive, checksums, copy_licenses,
                            copy_media_sources, copy_source, verify_media)

NAME = f'M4B-Editor-{VERSION}-linux-x86_64'
BINARIES = ('m4b-editor', 'm4b-editor-cli', 'ffmpeg', 'ffprobe')


def dependencies(work, payload):
    # dpkg-shlibdeps needs a source package identity even for an upstream build.
    debian = work/'debian'
    debian.mkdir(exist_ok=True)
    (debian/'control').write_text(
        'Source: m4b-editor\nSection: sound\nPriority: optional\n'
        'Maintainer: M4B Editor maintainers <aruel-perso@users.noreply.github.com>\n\n'
        'Package: m4b-editor\nArchitecture: amd64\nDescription: Audiobook editor\n')
    staged = payload.relative_to(work)
    result = subprocess.check_output(
        ['dpkg-shlibdeps', '-O', *(f'-e{staged/name}' for name in BINARIES)],
        cwd=work, text=True)
    depends = result.strip().removeprefix('shlibs:Depends=')
    # The display backends load these providers at runtime rather than linking to them.
    desktop_runtime = (
        'libegl1', 'libgl1', 'libwayland-client0', 'libxkbcommon0', 'libxkbcommon-x11-0',
        'libx11-6', 'libx11-xcb1', 'libxcursor1', 'libxi6', 'libxrandr2',
        'xdg-desktop-portal', 'xdg-utils',
    )
    return ', '.join(filter(None, (depends, *desktop_runtime)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=ROOT/f'dist/release-{VERSION}')
    parser.add_argument('--scratch', type=Path, default=Path(os.environ.get('TMPDIR', '/root/.hermes/cache/scratch')))
    args = parser.parse_args()
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        parser.error('Native Linux x86_64 required')
    media = ROOT/'vendor/linux-media'
    verify_media(media, 'linux-x86_64')
    target = 'x86_64-unknown-linux-gnu'
    subprocess.run(['cargo', 'build', '--locked', '--release', '--target', target,
                    '-p', 'm4b-editor-desktop', '-p', 'm4b-editor-cli'], cwd=ROOT, check=True)
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    scratch = args.scratch.resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='m4b-linux-', dir=scratch) as temporary:
        work = Path(temporary)
        package = work/'debian/m4b-editor'
        payload = package/'usr/lib/m4b-editor'
        payload.mkdir(parents=True)
        binaries = ROOT/'target'/target/'release'
        for name in BINARIES:
            shutil.copy2((binaries if name.startswith('m4b-editor') else media)/name, payload/name)
            (payload/name).chmod(0o755)
        shutil.copy2(ROOT/'packaging/linux/INSTALL.txt', payload)
        shutil.copy2(ROOT/'LICENSE', payload)
        copy_source(payload/'source')
        copy_licenses(payload)
        copy_media_sources(media, payload/'source')
        control = package/'DEBIAN'
        control.mkdir()
        depends = dependencies(work, payload)
        with (payload/'INSTALL.txt').open('a') as instructions:
            instructions.write(f'\nRuntime packages: {depends}\n')
        archive(payload, output/f'{NAME}.zip')
        for directory, source, name in [
            ('usr/share/applications', ROOT/'packaging/linux/m4b-editor.desktop', 'm4b-editor.desktop'),
            ('usr/share/pixmaps', ROOT/'packaging/icons/app.png', 'm4b-editor.png'),
            ('usr/share/doc/m4b-editor', ROOT/'LICENSE', 'copyright'),
        ]:
            destination = package/directory
            destination.mkdir(parents=True)
            shutil.copy2(source, destination/name)
        links = package/'usr/bin'
        links.mkdir(parents=True)
        for name in BINARIES[:2]:
            (links/name).symlink_to(f'../lib/m4b-editor/{name}')
        (control/'control').write_text(
            f'Package: m4b-editor\nVersion: {VERSION.replace("-", "~", 1)}\nArchitecture: amd64\n'
            'Section: sound\nPriority: optional\nMaintainer: M4B Editor maintainers <aruel-perso@users.noreply.github.com>\n'
            f'Depends: {depends}\nHomepage: https://github.com/aruel-perso/m4b-editor\n'
            'Description: Edit, chapter and export audiobooks\n')
        subprocess.run(['dpkg-deb', '--root-owner-group', '--build', package, output/f'{NAME}.deb'], check=True)
    checksums(output)


if __name__ == '__main__':
    main()
