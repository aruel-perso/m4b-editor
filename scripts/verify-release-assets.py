#!/usr/bin/env python3
"""Enforce exact flat channel assets, checksums and optionally pinned signatures."""
import argparse
import json
import subprocess
import tempfile
from pathlib import Path
from release_common import ROOT, VERSION, sha256, valid_version

def verify_envelope(file, installer, version, platform, legacy):
    envelope = json.loads(file.read_text())
    if set(envelope) != {'payload','signature'}: raise ValueError('Invalid envelope fields')
    payload = json.loads(envelope['payload'])
    repo = 'm4b-craft' if legacy else 'm4b-editor'
    expected = dict(version=version,platform=platform,filename=installer.name,
                    size=installer.stat().st_size,sha256=sha256(installer),
                    url=f'https://github.com/aruel-perso/{repo}/releases/download/v{version}/{installer.name}')
    if payload != expected: raise ValueError('Signed payload does not match exact release origin/content')
    signature = bytes.fromhex(envelope['signature'])
    if len(signature) != 64: raise ValueError('Invalid signature length')
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        (d/'payload').write_bytes(envelope['payload'].encode())
        (d/'signature').write_bytes(signature)
        (d/'key').write_bytes(bytes.fromhex('302a300506032b6570032100') + bytes.fromhex((ROOT/'packaging/update-public-key.hex').read_text().strip()))
        subprocess.run(['openssl','pkeyutl','-verify','-rawin','-pubin','-keyform','DER',
                        '-inkey',str(d/'key'),'-in',str(d/'payload'),'-sigfile',str(d/'signature')],
                       check=True,capture_output=True)

def verify(directory, version, legacy=False, signed=False):
    if not valid_version(version): raise ValueError('Invalid semver')
    names = set(); manifests = {}
    prefix = 'm4b-Craft' if legacy else 'M4B-Editor'
    platforms = [('windows-x86_64', ['msi']), ('macos-aarch64', ['dmg'])]
    if not legacy:
        platforms = [(platform, extensions + ['zip']) for platform, extensions in platforms]
        platforms.append(('linux-x86_64', ['deb', 'zip']))
    for platform, extensions in platforms:
        base = f'{prefix}-{version}-{platform}'
        names |= {base+'.'+ext for ext in extensions}
        if legacy:
            names.update({base+'-media-sources.zip', base+'-licenses.zip'})
        if signed:
            manifest = f'update-{platform}.json' if legacy else f'update-v2-{platform}.json'
            manifests[manifest] = (base+'.'+extensions[0], platform)
    expected = names | {name+'.sha256' for name in names} | set(manifests)
    actual = {p.name for p in directory.iterdir()}
    if actual != expected: raise ValueError(f'Asset set mismatch: missing={expected-actual}, extra={actual-expected}')
    for name in expected:
        file = directory/name
        if file.is_symlink() or not file.is_file(): raise ValueError(f'Non-flat asset: {file}')
    for name in names:
        file = directory/name
        if (directory/(name+'.sha256')).read_text() != sha256(file)+'  '+name+'\n':
            raise ValueError(f'Checksum mismatch: {name}')
        if not file.stat().st_size: raise ValueError(f'Empty asset: {name}')
    for name, (installer, platform) in manifests.items():
        verify_envelope(directory/name,directory/installer,version,platform,legacy)
    print('PASS exact flat assets and SHA-256 checksums' + (' and pinned signatures' if signed else ''))

if __name__ == '__main__':
    p = argparse.ArgumentParser(); p.add_argument('directory',type=Path); p.add_argument('--version',default=VERSION)
    p.add_argument('--legacy',action='store_true'); p.add_argument('--signed',action='store_true')
    a = p.parse_args(); verify(a.directory,a.version,a.legacy,a.signed)
