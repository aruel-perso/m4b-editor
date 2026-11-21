#!/usr/bin/env python3
"""Build native ARM64 app + drag-to-Applications DMG, with corresponding sources."""
import argparse
import os
import platform
import plistlib
import shutil
import subprocess
import tempfile
from pathlib import Path
from release_common import MEDIA_BUILD_FILES, ROOT, VERSION, SOURCES, verify_media, copy_source, copy_licenses, checksums, license_sidecar
from importlib.util import spec_from_file_location, module_from_spec

def load(name):
    spec = spec_from_file_location(name, ROOT/f'scripts/{name}.py')
    module = module_from_spec(spec); spec.loader.exec_module(module)
    return module

def install_app_icon(resources, info):
    icon = 'app.icns'
    shutil.copy2(ROOT/'packaging/icons'/icon, resources/icon)
    info['CFBundleIconFile'] = icon

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--legacy', action='store_true', help='Build exact old helper bundle and asset names')
    p.add_argument('--out',type=Path,default=ROOT/f'dist/release-{VERSION}')
    p.add_argument('--scratch',type=Path,default=Path(os.environ.get('TMPDIR','/root/.hermes/cache/scratch')))
    a = p.parse_args()
    if platform.system() != 'Darwin' or platform.machine() != 'arm64': p.error('Native macOS ARM64 required')
    os.environ['MACOSX_DEPLOYMENT_TARGET'] = '11.0'
    subprocess.run(['cargo','build','--locked','--release','--target','aarch64-apple-darwin','-p','m4b-editor-desktop','-p','m4b-editor-cli'],cwd=ROOT,check=True)
    media = ROOT/'vendor/macos-media'; verify_media(media,'macos-aarch64')
    a.out.mkdir(parents=True,exist_ok=True); a.scratch.mkdir(parents=True,exist_ok=True)
    output = a.out.resolve(); name = f'M4B-Editor-{VERSION}-macos-aarch64'
    if a.legacy: name = f'm4b-Craft-{VERSION}-macos-aarch64'
    with tempfile.TemporaryDirectory(prefix='m4b-macos-',dir=a.scratch) as temporary:
        stage = Path(temporary)/'disk'; stage.mkdir()
        app = stage/'M4B Editor.app'; contents = app/'Contents'
        binaries = contents/'MacOS'; binaries.mkdir(parents=True)
        resources = contents/'Resources'; resources.mkdir()
        for src,dest in [('m4b-editor','m4b-editor'),('m4b-editor-cli','m4b-editor-cli')]:
            shutil.copy2(ROOT/'target/aarch64-apple-darwin/release'/src,binaries/dest)
        for tool in ['ffmpeg','ffprobe']: shutil.copy2(media/tool,binaries/tool)
        for binary in binaries.iterdir():
            binary.chmod(0o755); load('verify-macos').verify(binary)
        info = dict(CFBundleName='M4B Editor',CFBundleDisplayName='M4B Editor',CFBundleExecutable='m4b-editor',
            CFBundleIdentifier='org.m4bcraft.desktop',CFBundlePackageType='APPL',
            CFBundleShortVersionString=VERSION.split('-')[0],CFBundleVersion=VERSION.split('-')[0],
            LSMinimumSystemVersion='11.0',NSHighResolutionCapable=True,
            CFBundleDocumentTypes=[dict(CFBundleTypeName='M4B Editor project',CFBundleTypeRole='Editor',
                LSHandlerRank='Owner',LSItemContentTypes=['org.m4bcraft.project'])],
            UTExportedTypeDeclarations=[dict(UTTypeIdentifier='org.m4bcraft.project',
                UTTypeDescription='M4B Editor project',UTTypeConformsTo=['public.json'],
                UTTypeTagSpecification={'public.filename-extension':['m4bproj']})])
        install_app_icon(resources, info)
        (contents/'Info.plist').write_bytes(plistlib.dumps(info))
        (contents/'PkgInfo').write_bytes(b'APPL????')
        copy_source(resources/'source'); copy_licenses(resources)
        shutil.copy2(ROOT/'LICENSE',resources/'LICENSE')
        license_sidecar(resources,media,output/f'{name}-licenses.zip')
        third = resources/'source/third-party/ffmpeg'; third.mkdir(parents=True)
        for file in MEDIA_BUILD_FILES:
            shutil.copy2(media/file, third/file)
        (resources/'MEDIA-SOURCES.txt').write_text(f'Corresponding FFmpeg/zlib/LAME source archives: {name}-media-sources.zip alongside this DMG. Build recipes/configuration: source/.\n')
        if a.legacy:
            shutil.copy2(binaries/'m4b-editor', binaries/'m4b-Craft')
            info['CFBundleExecutable'] = 'm4b-Craft'
            (contents/'Info.plist').write_bytes(plistlib.dumps(info))
            legacy_app = stage/'m4b-Craft.app'
            app.rename(legacy_app)
            app = legacy_app
            binaries = app/'Contents/MacOS'
        # Sign inner executables first, then seal the bundle. No Developer ID/notarization.
        for binary in binaries.iterdir(): subprocess.run(['codesign','--force','--sign','-',str(binary)],check=True)
        subprocess.run(['codesign','--force','--sign','-',str(app)],check=True)
        subprocess.run(['codesign','--verify','--deep','--strict',str(app)],check=True)
        (stage/'Applications').symlink_to('/Applications')
        shutil.copy2(ROOT/'packaging/macos/INSTALL.txt',stage/'INSTALL.txt')
        sidecar = Path(temporary)/'media-source'; sidecar.mkdir()
        for source in SOURCES: shutil.copy2(media/source,sidecar/source)
        load('package-release').archive(sidecar,output/f'{name}-media-sources.zip')
        dmg = output/f'{name}.dmg'
        if dmg.exists(): dmg.unlink()
        subprocess.run(['hdiutil','create','-volname','M4B Editor','-srcfolder',str(stage),'-format','UDZO','-ov',str(dmg)],check=True)
    checksums(output)
    print(output)
if __name__ == '__main__': main()
