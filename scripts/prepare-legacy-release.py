#!/usr/bin/env python3
"""Prepare local release-only old-slug assets. Never uploads or signs implicitly."""
import argparse
from pathlib import Path
import shutil
import zipfile
from release_common import VERSION, MEDIA_LICENSES, SOURCES, checksums, valid_version

def prepare(canonical, legacy_dmg, output, version):
    if not valid_version(version): raise ValueError('Invalid semver')
    if output.is_symlink(): raise ValueError('Symlinked output')
    if output.exists() and any(output.iterdir()):
        raise ValueError('Compatibility output must be empty')
    output.mkdir(parents=True, exist_ok=True)
    # Windows installation bytes are identical, including canonical and legacy exe.
    windows = f'M4B-Editor-{version}-windows-x86_64.msi'
    mac = f'm4b-Craft-{version}-macos-aarch64.dmg'
    if legacy_dmg.name != mac or legacy_dmg.is_symlink():
        raise ValueError('Expected separately built legacy DMG, never rename canonical DMG')
    for source, name in [(canonical/windows, windows.replace('M4B-Editor','m4b-Craft')),
                         (legacy_dmg, mac)]:
        if source.is_symlink() or not source.is_file() or not source.stat().st_size:
            raise ValueError(f'Invalid installer: {source}')
        shutil.copy2(source, output/name)
    # Older releases provided sidecars. Current Windows packages include them as
    # source archives and license files inside the portable payload.
    windows_base = windows.removesuffix('.msi')
    for suffix in ('media-sources', 'licenses'):
        name = f'{windows_base}-{suffix}.zip'
        destination = output/name.replace('M4B-Editor','m4b-Craft')
        if (canonical/name).is_file():
            shutil.copy2(canonical/name, destination)
        else:
            with zipfile.ZipFile(canonical/f'{windows_base}.zip') as portable:
                with zipfile.ZipFile(destination, 'w', zipfile.ZIP_DEFLATED) as archive:
                    media = 'source/third-party/ffmpeg/'
                    if suffix == 'media-sources':
                        members = [(media + source, source) for source in SOURCES]
                    else:
                        members = [(media + license, 'media/' + license) for license in MEDIA_LICENSES]
                        members += [(path, path) for path in portable.namelist()
                                    if path.startswith('licenses/') or path in ('LICENSE', 'THIRD-PARTY-NOTICES.txt')]
                    for source, member in members:
                        archive.writestr(member, portable.read(source))
        legacy_sidecar = legacy_dmg.with_name(f'{legacy_dmg.stem}-{suffix}.zip')
        old_sidecar = canonical/legacy_sidecar.name.replace('m4b-Craft','M4B-Editor')
        shutil.copy2(legacy_sidecar if legacy_sidecar.is_file() else old_sidecar, output/legacy_sidecar.name)
    checksums(output)

if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--canonical', type=Path, required=True)
    p.add_argument('--legacy-dmg', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--version', default=VERSION)
    a = p.parse_args(); prepare(a.canonical,a.legacy_dmg,a.out,a.version)
