#!/usr/bin/env python3
"""Sign an update envelope using an external Ed25519 PEM (never copied).
Python stdlib + OpenSSL 3 CLI. Payload is signed as exact UTF-8 JSON bytes.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--legacy', action='store_true', help='Sign old-slug, old-name compatibility channel')
    p.add_argument('--key', type=Path, required=True)
    installers = p.add_mutually_exclusive_group(required=True)
    installers.add_argument('--installer', type=Path)
    installers.add_argument('--msi', type=Path, help='Backward compatible Windows installer alias')
    p.add_argument('--platform', choices=['windows-x86_64','macos-aarch64','linux-x86_64'], default='windows-x86_64')
    p.add_argument('--version', required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--scratch', type=Path, required=True)
    a = p.parse_args()
    if a.key.resolve().is_relative_to(ROOT): p.error('Signing key must be outside repository')
    match = re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?', a.version)
    if not match or (match[4] and any(part.isdigit() and len(part)>1 and part[0]=='0' for part in match[4].split('.'))):
        p.error('Version must be canonical semver without build metadata')
    if a.msi and a.platform != 'windows-x86_64': p.error('--msi requires windows-x86_64')
    a.installer = a.installer or a.msi
    if a.legacy and a.platform == 'linux-x86_64': p.error('No legacy Linux update channel')
    extension = {'windows-x86_64':'msi', 'macos-aarch64':'dmg', 'linux-x86_64':'deb'}[a.platform]
    prefix_name = 'm4b-Craft' if a.legacy else 'M4B-Editor'
    repository = 'm4b-craft' if a.legacy else 'm4b-editor'
    expected = f'{prefix_name}-{a.version}-{a.platform}.{extension}'
    manifest_name = f'update-{a.platform}.json' if a.legacy else f'update-v2-{a.platform}.json'
    if a.out.name != manifest_name: p.error(f'Expected manifest filename {manifest_name}')
    if a.installer.is_symlink(): p.error('Symlinked installer')
    if a.installer.name != expected: p.error(f'Expected filename {expected}')
    if not 0 < a.installer.stat().st_size <= 512*1024*1024: p.error('Invalid installer size')
    public_der = subprocess.check_output(['openssl','pkey','-in',str(a.key),'-pubout','-outform','DER'])
    # Ed25519 SubjectPublicKeyInfo prefix followed by exactly 32 raw bytes.
    prefix = bytes.fromhex('302a300506032b6570032100')
    expected_key = bytes.fromhex((ROOT/'packaging/update-public-key.hex').read_text().strip())
    if public_der != prefix + expected_key: p.error('PEM does not match pinned public key')
    with a.installer.open('rb') as file:
        checksum = hashlib.file_digest(file, 'sha256').hexdigest()
    payload = json.dumps(dict(version=a.version, platform=a.platform,filename=expected,
        sha256=checksum,size=a.installer.stat().st_size,
        url=f'https://github.com/aruel-perso/{repository}/releases/download/v{a.version}/{expected}'),
        separators=(',',':'),sort_keys=True)
    a.scratch.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='m4b-sign-',dir=a.scratch) as temporary:
        path = Path(temporary)/'payload'
        path.write_bytes(payload.encode())
        signature = subprocess.check_output(['openssl','pkeyutl','-sign','-rawin','-inkey',str(a.key),'-in',str(path)])
        if len(signature) != 64: p.error('Expected Ed25519 signature')
        sig = Path(temporary)/'signature'; sig.write_bytes(signature)
        pub = Path(temporary)/'public.der'; pub.write_bytes(public_der)
        subprocess.run(['openssl','pkeyutl','-verify','-rawin','-pubin','-keyform','DER','-inkey',str(pub),'-in',str(path),'-sigfile',str(sig)],check=True)
    a.out.write_text(json.dumps(dict(payload=payload,signature=signature.hex()),separators=(',',':'))+'\n', encoding='utf-8')

if __name__ == '__main__': main()
