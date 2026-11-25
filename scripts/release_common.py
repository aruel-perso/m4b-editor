"""Shared release contract; no network, build or platform commands at import time."""
import hashlib
import re
import shutil
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VERSION = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
SEMVER = r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?'
def valid_version(version):
    match = re.fullmatch(SEMVER, version)
    return bool(match and not (match[4] and any(p.isdigit() and len(p)>1 and p[0]=='0' for p in match[4].split('.'))))
if not valid_version(VERSION):
    raise ValueError('Noncanonical workspace version')
SOURCES = {
    'lame-3.100.tar.gz': 'ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e',
    'ffmpeg-8.0.1.tar.xz': '05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41',
    'zlib-1.3.1.tar.gz': '9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23',
}
LEGACY_ARCHIVER_SOURCES = {
    '7z2606-x64.exe': '9b697f65ecc2d9b0152f5ee96d72dde522a7827690349d9307935dacf40f42ec',
    '7z2606-mac.tar.xz': 'b6f17920b4240940461dac8ef4009891881ac44d2a5ead625ede2ec8472185dd',
    '7z2606-src.tar.xz': 'a4e374c8f864f51df48c2cfd0e1413ec3b2ea030deae852d6a655402ebb61ac4',
}
LEGACY_ARCHIVER_MEMBERS = {
    'windows-x86_64': {
        '7z.exe': 'dc0493429fe28136fcb177a0e77c9875cbb3b0bff5f2f8eeba4df302e98cef76',
        '7z.dll': '500db9eb17f3c21b729ba41dd02ecfe1823da05028c7e4ba129084cc74cc364e',
        'LICENSE.7zip': '519ac0a4bded9c18ea02e0afb71f663d8c47373bd9facd3ac96a79f51d77765d',
        'README.7zip': 'de4a45e9dcdabfd21e5e0e31e29079998f216db15079217b6160e92b1c151e40',
    },
    'macos-aarch64': {
        '7zz': '56cf8eb2fdc6f9c0ac5d7a3cd238505814ac8ed6b88a7c64a7e3256036252c60',
        'LICENSE.7zip': '1790374e5352329cedb46ee3808930a88e9ca2f08b82b10fcf5cf605d2c301b1',
        'README.7zip': '79aa0fedbfb43f32d60858c8f13f42afc025adf19bf3e7ed66a7c55f451eae45',
    },
}
MEDIA_LICENSES = ('COPYING.LGPLv2.1', 'LICENSE.md', 'LICENSE.zlib', 'COPYING.LAME')
MEDIA_BUILD_FILES = MEDIA_LICENSES + ('config.h', 'config_components.h', 'config.mak',
                                     'zlib-config.txt', 'lame-config.txt')

def sha256(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()
def verify_media(media, platform):
    executables = ['ffmpeg.exe','ffprobe.exe'] if platform == 'windows-x86_64' else ['ffmpeg','ffprobe']
    expected = set(executables) | set(SOURCES) | set(MEDIA_BUILD_FILES)
    # Old, complete pinned caches can supply the same FFmpeg build. Package
    # callers use the explicit media file lists and never copy these extras.
    legacy = (LEGACY_ARCHIVER_SOURCES | LEGACY_ARCHIVER_MEMBERS[platform]
              if platform in LEGACY_ARCHIVER_MEMBERS else {})
    actual = {p.name for p in media.iterdir()} - {'SHA256SUMS'}
    if actual == expected | legacy.keys():
        expected |= legacy.keys()
        pins = SOURCES | legacy
    elif actual == expected:
        pins = SOURCES
    else:
        raise ValueError(f'Unexpected media cache contents: {actual ^ expected}')
    if (media/'SHA256SUMS').is_symlink():
        raise ValueError('Symlinked media checksum manifest')
    records = {}
    for line in (media/'SHA256SUMS').read_text().splitlines():
        digest, name = line.split(maxsplit=1)
        name = name.lstrip('*')
        if Path(name).name != name or name in records or not re.fullmatch('[a-f0-9]{64}', digest):
            raise ValueError('Invalid media checksum record')
        records[name] = digest
    if set(records) != expected:
        raise ValueError(f'Unexpected media cache contents: {set(records) ^ expected}')
    for name, digest in records.items():
        if (media/name).is_symlink() or sha256(media/name) != digest:
            raise ValueError(f'Media checksum mismatch: {name}')
    for name, digest in pins.items():
        if records[name] != digest:
            raise ValueError(f'Unpinned source: {name}')
def copy_source(destination):
    destination.mkdir(parents=True, exist_ok=True)
    for name in ['crates','.cargo','.github','scripts','packaging','docs','tests']:
        if name == 'tests' and not (ROOT/name).exists(): continue
        for entry in (ROOT/name).rglob('*'):
            if entry.is_symlink() or entry.suffix.lower() in ('.pem','.key'):
                raise ValueError(f'Refusing private key or symlink: {entry}')
        shutil.copytree(ROOT/name, destination/name, ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
    for name in ['Cargo.toml','Cargo.lock','.gitignore','.gitattributes','LICENSE','README.md','README.en.md']:
        shutil.copy2(ROOT/name, destination/name)
def copy_media_sources(media, source_directory):
    destination = source_directory/'third-party/ffmpeg'
    destination.mkdir(parents=True)
    for name in (*MEDIA_BUILD_FILES, *SOURCES):
        target = destination/name
        shutil.copy2(media/name, target)
        # Verified downloads can be mode 0600. Root-owned installations must
        # expose corresponding sources/licenses to ordinary application users.
        target.chmod(0o644)
    return destination

def verify_source_archives(directory):
    for name, digest in SOURCES.items():
        if sha256(directory/name) != digest:
            raise ValueError(f'Bundled media source checksum mismatch: {name}')

def copy_licenses(destination):
    import json
    metadata = json.loads(subprocess.check_output(['cargo','metadata','--locked','--offline','--format-version=1'],cwd=ROOT))
    notices = []
    for package in metadata['packages']:
        if not package.get('source'): continue
        notices.append(f"{package['name']} {package['version']}: {package.get('license')}")
        base = Path(package['manifest_path']).parent
        candidates = [file for file in base.iterdir() if file.is_file() and
                      file.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE',
                                                     'COPYRIGHT', 'THIRD_PARTY_LICENSES'))]
        if package.get('license_file'):
            candidates.append(base/package['license_file'])
        if (base/'LICENSES').is_dir():
            candidates.extend(file for file in (base/'LICENSES').rglob('*') if file.is_file())
        for file in sorted(set(candidates)):
            dest = destination/'licenses'/f"{package['name']}-{package['version']}"/file.relative_to(base)
            dest.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(file,dest)
    (destination/'THIRD-PARTY-NOTICES.txt').write_text('\n'.join(sorted(notices))+'\n',encoding='utf-8')
def license_sidecar(payload, media, output):
    """Standalone redistributable license notices; also retained inside installers."""
    import zipfile
    files = [(payload/'LICENSE', 'LICENSE'),
             (payload/'THIRD-PARTY-NOTICES.txt', 'THIRD-PARTY-NOTICES.txt')]
    files += [(p, p.relative_to(payload).as_posix()) for p in sorted((payload/'licenses').rglob('*')) if p.is_file()]
    files += [(media/name, 'media/'+name) for name in
              MEDIA_LICENSES]
    with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED,strict_timestamps=False) as archive:
        for file, name in files:
            if file.is_symlink(): raise ValueError(f'Symlinked license: {file}')
            archive.write(file,name)

def archive(directory, output):
    import zipfile
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED, strict_timestamps=False) as zipped:
        for file in sorted(directory.rglob('*')):
            if file.is_file():
                zipped.write(file, file.relative_to(directory))

def checksums(output):
    for file in sorted(output.iterdir()):
        if file.suffix in ('.msi','.dmg','.deb','.zip'):
            file.with_name(file.name+'.sha256').write_text(sha256(file)+'  '+file.name+'\n')
if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--media',type=Path)
    parser.add_argument('--platform',choices=['windows-x86_64','macos-aarch64','linux-x86_64'])
    args = parser.parse_args()
    if args.media:
        if not args.platform: parser.error('--platform required with --media')
        verify_media(args.media,args.platform)
    else: print(VERSION)
