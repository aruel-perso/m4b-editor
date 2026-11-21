#!/usr/bin/env python3
"""Build the Windows release payload and WiX MSI; never uses old alpha artifacts.
Requires freshly built Windows GNU release binaries, wixl, msibuild and locked Cargo sources.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import uuid
import zipfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
from release_common import MEDIA_BUILD_FILES, SOURCES, VERSION, verify_media, copy_source, copy_licenses, checksums, license_sidecar
from windows_msi_ui import add_wizard

MSI_VERSION = VERSION.split('-')[0]
UPGRADE_CODE = '790fce8b-e564-43b1-9701-d427fdd80c6c'
NAME = f'M4B-Editor-{VERSION}-windows-x86_64'
SHORTCUT_ICON = 'M4BEditorIcon.exe'
NS = 'http://schemas.microsoft.com/wix/2006/wi'
ET.register_namespace('', NS)

def node(parent, tag, **attrs):
    return ET.SubElement(parent, '{'+NS+'}'+tag, attrs)

def build_shortcut_icon(output):
    # Advertised shortcuts require a small PE icon file with the target's extension.
    with tempfile.TemporaryDirectory(dir=output.parent) as temporary:
        resource = Path(temporary).resolve() / 'app-icon.o'
        subprocess.run(['x86_64-w64-mingw32-windres', '-O', 'coff', '-i', 'app.rc',
                        '-o', str(resource)], cwd=ROOT/'packaging/icons', check=True)
        subprocess.run(['x86_64-w64-mingw32-gcc', '-nostdlib', '-Wl,--entry,0,--no-insert-timestamp',
                        '-o', str(output), str(resource)], check=True)

def build_legacy_launcher(output):
    subprocess.run([
        'x86_64-w64-mingw32-gcc', '-municode', '-mwindows', '-Os', '-Wall', '-Wextra',
        '-Wl,--no-insert-timestamp', str(ROOT/'packaging/windows/legacy-launch.c'),
        '-o', str(output), '-ladvapi32',
    ], check=True)
    verify_pe(output)


def wix(payload, output, shortcut_icon, legacy_launcher=None):
    root = ET.Element('{'+NS+'}Wix')
    product = node(root, 'Product', Id=str(uuid.uuid5(uuid.UUID(UPGRADE_CODE), VERSION)),
                   Name='M4B Editor', Language='1033', Version=MSI_VERSION,
                   Manufacturer='M4B Editor contributors', UpgradeCode=UPGRADE_CODE)
    node(product, 'Package', InstallerVersion='500', Compressed='yes', InstallScope='perUser',
         Description='M4B Editor audiobook editor')
    node(product, 'Icon', Id=SHORTCUT_ICON, SourceFile=os.path.relpath(shortcut_icon, output.parent))
    node(product, 'Media', Id='1', Cabinet='application.cab', EmbedCab='yes')
    node(product, 'MajorUpgrade', DowngradeErrorMessage='A newer M4B Editor is already installed.')
    sequence = node(product, 'InstallExecuteSequence')
    node(sequence, 'RemoveExistingProducts', After='InstallInitialize')
    directory = node(product, 'Directory', Id='TARGETDIR', Name='SourceDir')
    local = node(directory, 'Directory', Id='LocalAppDataFolder')
    programs = node(local, 'Directory', Id='Programs', Name='Programs')
    install = node(programs, 'Directory', Id='INSTALLDIR', Name='M4B Editor')
    menu = node(directory, 'Directory', Id='ProgramMenuFolder')
    menu_dir = node(menu, 'Directory', Id='ApplicationProgramsFolder', Name='M4B Editor')
    feature = node(product, 'Feature', Id='Complete', Title='M4B Editor', Level='1')
    menu_component = node(menu_dir, 'Component', Id='UserRegistration',
        Guid=str(uuid.uuid5(uuid.UUID(UPGRADE_CODE), 'editor-user-registration')), Win64='yes')
    node(menu_component, 'RegistryValue', Root='HKCU', Key='Software\\M4B-Editor',
        Name='Installed', Type='integer', Value='1', KeyPath='yes')
    node(menu_component, 'RemoveFolder', Id='RemoveStartMenu', On='uninstall')
    node(feature, 'ComponentRef', Id='UserRegistration')
    if legacy_launcher is not None:
        legacy_install = node(programs, 'Directory', Id='LegacyRelaunchFolder', Name='m4b-Craft')
        for alias in ['m4b-Craft.exe', 'm4b-editor.exe']:
            component_id = 'LegacyRelaunch' + ('Craft' if 'Craft' in alias else 'Editor')
            bridge = node(legacy_install, 'Component', Id=component_id,
                          Guid=str(uuid.uuid5(uuid.UUID(UPGRADE_CODE), component_id)), Win64='yes')
            node(bridge, 'File', Id=component_id+'File', Source=str(legacy_launcher),
                 Name=alias, KeyPath='yes')
            node(feature, 'ComponentRef', Id=component_id)
    directories = {Path('.'): install}
    for index, file in enumerate(sorted(payload.rglob('*'))):
        if not file.is_file():
            continue
        relative = file.relative_to(payload)
        path = Path('.')
        for part in relative.parent.parts:
            parent = path
            path = path / part
            if path not in directories:
                directories[path] = node(directories[parent], 'Directory',
                    Id='D'+hashlib.sha256(str(path).encode()).hexdigest()[:24], Name=part)
        component_id = f'C{index}'
        # Components for moved resources must have new IDs. Keep the UpgradeCode
        # and the unmoved registration component stable.
        component = node(directories[relative.parent], 'Component', Id=component_id,
                         Guid=str(uuid.uuid5(uuid.UUID(UPGRADE_CODE), 'M4B Editor/'+relative.as_posix())),
                         Win64='yes')
        editor = relative.as_posix() == 'm4b-editor.exe'
        file_id = 'EditorExecutable' if editor else f'F{index}'
        installed = node(component, 'File', Id=file_id, Source=str(file), Name=file.name, KeyPath='yes')
        if editor:
            node(installed, 'Shortcut', Id='StartMenuShortcut', Directory='ApplicationProgramsFolder',
                 Name='M4B Editor', WorkingDirectory='INSTALLDIR', Advertise='yes', Icon=SHORTCUT_ICON, IconIndex='0')
            node(menu_component, 'RegistryValue', Root='HKCU', Key='Software\\M4B-Editor',
                 Name='Executable', Type='string', Value='[#EditorExecutable]')
            progid = 'M4BEditor.Project'
            classes = 'Software\\Classes\\'
            node(menu_component, 'RegistryValue', Root='HKCU', Key=classes+progid,
                 Type='string', Value='M4B Editor project')
            node(menu_component, 'RegistryValue', Root='HKCU', Key=classes+progid+'\\DefaultIcon',
                 Type='string', Value='[#EditorExecutable],0')
            node(menu_component, 'RegistryValue', Root='HKCU', Key=classes+progid+'\\shell\\open\\command',
                 Type='string', Value='"[#EditorExecutable]" "%1"')
            node(menu_component, 'RegistryValue', Root='HKCU', Key=classes+'.m4bproj\\OpenWithProgids',
                 Name=progid, Type='string', Value='')
        node(feature, 'ComponentRef', Id=component_id)
    ET.indent(root)
    ET.ElementTree(root).write(output, encoding='utf-8', xml_declaration=True)

def verify_pe(file):
    system = set(('advapi32 avrt bcrypt bcryptprimitives crypt32 d3d11 d3d12 d3dcompiler_47 '
        'dcomp dwmapi dxgi gdi32 imm32 iphlpapi kernel32 msvcrt ntdll ole32 oleaut32 '
        'opengl32 propsys psapi rpcrt4 secur32 setupapi shell32 shlwapi user32 userenv '
        'uuid version winhttp winmm ws2_32 wtsapi32 ucrtbase uiautomationcore uxtheme '
        'comdlg32 comctl32 msimg32 wintrust normaliz dnsapi netapi32').split())
    imports = subprocess.check_output(['x86_64-w64-mingw32-objdump','-p',str(file)],text=True)
    names = re.findall(r'DLL Name: (\S+)',imports)
    if not names: raise SystemExit(f'No PE imports found: {file}')
    for name in names:
        lower = name.lower()
        if not (lower.startswith(('api-ms-win-','ext-ms-win-')) or lower.removesuffix('.dll') in system):
            raise SystemExit(f'Unbundled non-system DLL in {file.name}: {name}')


def archive(directory, output):
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED, strict_timestamps=False) as z:
        for file in sorted(directory.rglob('*')):
            if file.is_file(): z.write(file, file.relative_to(directory))

def build_msi(wxs, msi, wixl, msibuild):
    # Stage beside the output so a failed compile or UI import cannot expose
    # a partially built MSI, and the final rename stays on one filesystem.
    with tempfile.TemporaryDirectory(prefix='msi-', dir=msi.parent) as staging:
        directory = Path(staging).resolve()
        candidate = directory / msi.name
        subprocess.run([wixl, '-a', 'x64', '-o', str(candidate), str(wxs)], check=True)
        add_wizard(candidate, msibuild, directory, ROOT/'packaging/icons/app.ico')
        candidate.replace(msi)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wixl', default='wixl')
    parser.add_argument('--msibuild', default='msibuild')
    parser.add_argument('--media', type=Path, default=ROOT/'vendor/windows-media')
    parser.add_argument('--out', type=Path, default=ROOT/f'dist/release-{VERSION}')
    parser.add_argument('--scratch', type=Path, default=Path(os.environ.get('TMPDIR','/root/.hermes/cache/scratch')))
    args = parser.parse_args()
    for tool in (args.wixl, args.msibuild):
        if not shutil.which(tool):
            parser.error(f'MSI packaging tool not found: {tool}')
    version = subprocess.check_output(['cargo','metadata','--no-deps','--format-version=1'], cwd=ROOT)
    assert all(p['version'] == VERSION for p in json.loads(version)['packages'])
    binaries = ROOT/'target/x86_64-pc-windows-gnu/release'
    # Always build first. A failed build must not silently package stale alpha executables.
    subprocess.run(['cargo','build','--locked','--release','--target','x86_64-pc-windows-gnu',
                    '-p','m4b-editor-desktop','-p','m4b-editor-cli'], cwd=ROOT, check=True)
    args.out.mkdir(parents=True, exist_ok=True)
    output = args.out.resolve()
    args.scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='m4b-release-', dir=args.scratch) as temporary:
        payload = Path(temporary)/'payload'
        payload.mkdir()
        for source, target in [('m4b-editor.exe','m4b-editor.exe'),('m4b-editor-cli.exe','m4b-editor-cli.exe')]:
            shutil.copy2(binaries/source, payload/target)
        media = args.media.resolve()
        verify_media(media, 'windows-x86_64')
        for name in ['ffmpeg.exe','ffprobe.exe']:
            shutil.copy2(media/name, payload/name)
        for executable in list(payload.glob('*.exe')) + list(payload.glob('*.dll')): verify_pe(executable)
        shutil.copy2(ROOT/'LICENSE', payload/'LICENSE')
        for name in ['Lancer-M4B-Editor.cmd','LISEZ-MOI-WINDOWS.txt']:
            shutil.copy2(ROOT/'packaging/windows'/name, payload/name)
        source = payload/'source'
        copy_source(source)
        third = source/'third-party/ffmpeg'
        third.mkdir(parents=True)
        for file in MEDIA_BUILD_FILES:
            shutil.copy2(media/file, third/file)
        # Upstream source sidecar ships alongside MSI; do not omit it from releases.
        upstream = Path(temporary)/'media-source'
        upstream.mkdir()
        for name in SOURCES:
            shutil.copy2(media/name, upstream/name)
        archive(upstream, output/f'{NAME}-media-sources.zip')
        (payload/'MEDIA-SOURCES.txt').write_text(f'Corresponding FFmpeg/zlib/LAME source archives: {NAME}-media-sources.zip, supplied alongside this MSI. Build recipe and configuration: source/.\n')
        copy_licenses(payload)
        license_sidecar(payload,media,output/f'{NAME}-licenses.zip')
        retained = output/'payload'
        if retained.exists(): shutil.rmtree(retained)
        shutil.copytree(payload, retained)
        wxs = output/'M4B-Editor.wxs'
        shortcut_icon = output/SHORTCUT_ICON
        build_shortcut_icon(shortcut_icon)
        legacy_launcher = output/'M4BEditorLegacyLaunch.exe'
        build_legacy_launcher(legacy_launcher)
        wix(retained, wxs, shortcut_icon, legacy_launcher)
        build_msi(wxs, output/f'{NAME}.msi', args.wixl, args.msibuild)
        archive(payload,output/f'{NAME}.zip')
    # Retained payload and generated WiX source allow MSI inspection/rebuilding.
    checksums(output)
    print(output)

if __name__ == '__main__': main()
