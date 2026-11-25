#!/usr/bin/env python3
"""Exercise release packaging/signing and optionally inspect/extract an actual MSI."""
import argparse
import importlib.util
import json
import plistlib
import struct
import xml.etree.ElementTree as ET
import os
import hashlib
import shutil
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile
from unittest.mock import patch
from release_common import verify_media, SOURCES, MEDIA_BUILD_FILES, LEGACY_ARCHIVER_SOURCES, LEGACY_ARCHIVER_MEMBERS, sha256

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('release', ROOT/'scripts/package-release.py')
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)

def verify_pe_icon(path):
    resources = subprocess.check_output(['x86_64-w64-mingw32-objdump', '-p', str(path)], text=True)
    tree = resources.split('The .rsrc Resource Directory section:', 1)[1]
    assert 'Entry: ID: 0x000003,' in tree  # RT_ICON
    assert 'Entry: ID: 0x00000e,' in tree  # RT_GROUP_ICON
    assert (ROOT/'packaging/icons/app.png').read_bytes() in path.read_bytes()


class ReleaseTests(unittest.TestCase):
    def test_linux_runtime_dependencies_include_dynamically_loaded_xkb(self):
        spec = importlib.util.spec_from_file_location('linux_package', ROOT/'scripts/package-linux.py')
        assert spec is not None and spec.loader is not None
        linux = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(linux)
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            work = Path(d)
            payload = work/'payload'
            payload.mkdir()
            with patch.object(linux.subprocess, 'check_output', return_value='shlibs:Depends=libc6 (>= 2.39)\n'):
                depends = linux.dependencies(work, payload)
            self.assertIn('libc6 (>= 2.39)', depends)
            self.assertIn('libxkbcommon-x11-0', depends.split(', '))

    def test_windows_shortcut_icon_pe(self):
        for tool in ['windres', 'gcc', 'objdump']:
            if not shutil.which(f'x86_64-w64-mingw32-{tool}'):
                self.skipTest('MinGW resource tools required')
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            icon = Path(d)/release.SHORTCUT_ICON
            release.build_shortcut_icon(icon)
            verify_pe_icon(icon)
            release.build_legacy_launcher(Path(d)/'legacy-launch.exe')

    def test_ico_contains_the_expected_png_sizes(self):
        ico = (ROOT/'packaging/icons/app.ico').read_bytes()
        sizes = [16, 24, 32, 48, 64, 128, 256]
        self.assertEqual(struct.unpack_from('<HHH', ico), (0, 1, len(sizes)))
        end = 6 + 16 * len(sizes)
        for index, size in enumerate(sizes):
            width, height, _, _, planes, depth, length, start = struct.unpack_from('<BBBBHHII', ico, 6 + 16 * index)
            self.assertEqual((width, height, planes, depth), (size % 256, size % 256, 1, 32))
            self.assertEqual(start, end)
            end = start + length
            png = ico[start:end]
            self.assertEqual(png[:16], b'\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR')
            self.assertEqual(struct.unpack_from('>II', png, 16), (size, size))
            if size == 64:
                self.assertEqual(png, (ROOT/'packaging/icons/app.png').read_bytes())
        self.assertEqual(end, len(ico))

    def test_mac_bundle_icon_resource(self):
        spec = importlib.util.spec_from_file_location('macos', ROOT/'scripts/package-macos.py')
        macos = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(macos)
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            resources = Path(d)/'Contents/Resources'
            resources.mkdir(parents=True)
            info = {}
            macos.install_app_icon(resources, info)
            plist = resources.parent/'Info.plist'
            plist.write_bytes(plistlib.dumps(info))
            icon = resources/plistlib.loads(plist.read_bytes())['CFBundleIconFile']
            data = icon.read_bytes()
            self.assertEqual(data, (ROOT/'packaging/icons/app.icns').read_bytes())
            self.assertEqual(struct.unpack_from('>4sI', data), (b'icns', len(data)))
            cursor = 8
            sizes = {}
            while cursor < len(data):
                kind, length = struct.unpack_from('>4sI', data, cursor)
                self.assertGreaterEqual(length, 8 + 24)
                self.assertLessEqual(cursor + length, len(data))
                self.assertEqual(data[cursor+8:cursor+16], b'\x89PNG\r\n\x1a\n')
                sizes[kind] = struct.unpack_from('>II', data, cursor + 24)
                cursor += length
            for kind, size in [(b'icp4', 16), (b'ic07', 128), (b'ic10', 1024)]:
                self.assertEqual(sizes[kind], (size, size))

    def test_copy_licenses_accepts_null_license_file(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d)
            package = root/'crate'; package.mkdir()
            (package/'Cargo.toml').write_text('[package]')
            (package/'README.md').write_text('not a license')
            (package/'LICENSE-MIT').write_text('fixture MIT license')
            out = root/'payload'; out.mkdir()
            metadata = {'packages': [{'name': 'fixture', 'version': '1.0.0',
                'source': 'registry+fixture', 'license': 'MIT', 'license_file': None,
                'manifest_path': str(package/'Cargo.toml')}]}
            with patch('release_common.subprocess.check_output', return_value=json.dumps(metadata).encode()):
                release.copy_licenses(out)
            self.assertEqual((out/'licenses/fixture-1.0.0/LICENSE-MIT').read_text(), 'fixture MIT license')
            self.assertFalse((out/'licenses/fixture-1.0.0/README.md').exists())
            self.assertIn('fixture 1.0.0: MIT', (out/'THIRD-PARTY-NOTICES.txt').read_text())

    @unittest.skipIf(os.name == 'nt', 'POSIX payload permission regression')
    def test_media_sources_remain_readable_after_root_owned_install(self):
        from release_common import copy_media_sources
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d)
            media = root/'media'; media.mkdir()
            for name in (*MEDIA_BUILD_FILES, *SOURCES):
                file = media/name
                file.write_bytes(b'synthetic source fixture')
                file.chmod(0o600)
            source = copy_media_sources(media, root/'source')
            for file in source.iterdir():
                self.assertEqual(file.stat().st_mode & 0o777, 0o644, file.name)

    def test_source_copy_excludes_private_notes_and_includes_tests(self):
        from release_common import copy_source
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            out = Path(d)/'source'
            copy_source(out)
            for name in ['BRIEF.md','VALIDATION.md','ALPHA-0.1.1.md','implementation-plan.md',
                         'tests-local']:
                self.assertFalse((out/name).exists(), name)
            self.assertTrue((out/'docs/development.md').is_file())
            self.assertTrue((out/'crates').is_dir())
            if (ROOT/'tests').exists(): self.assertTrue((out/'tests').is_dir())

    def test_layout_identity(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            path = Path(d)
            payload = path/'payload'; payload.mkdir()
            (payload/'m4b-editor.exe').write_bytes(b'fixture')
            bridge = path/'bridge.exe'; bridge.write_bytes(b'fixture')
            source = payload/'source'; source.mkdir(); (source/'Cargo.lock').write_text('fixture')
            icon = path/release.SHORTCUT_ICON
            icon.write_bytes(b'fixture')
            wxs = path/'app.wxs'; release.wix(payload,wxs,icon,bridge)
            product = ET.parse(wxs).getroot().find(f'{{{release.NS}}}Product')
            icon_node = product.find(f'{{{release.NS}}}Icon')
            self.assertEqual(wxs.parent/icon_node.attrib['SourceFile'], icon)
            shortcuts = product.findall(f'.//{{{release.NS}}}Shortcut')
            self.assertEqual({shortcut.attrib['Id'] for shortcut in shortcuts},
                             {'StartMenuShortcut'})
            for shortcut in shortcuts:
                self.assertEqual(shortcut.attrib['Icon'], icon_node.attrib['Id'])
                self.assertEqual(shortcut.attrib['IconIndex'], '0')
                self.assertEqual(shortcut.attrib['Advertise'], 'yes')
            text = wxs.read_text()
            self.assertIn(release.UPGRADE_CODE,text)
            self.assertIn(f'Version="{release.MSI_VERSION}"',text)
            self.assertIn('InstallScope="perUser"',text)
            self.assertIn('MajorUpgrade',text)
            self.assertNotIn('RemoveFile',text)
            self.assertIn('Cargo.lock',text)
            self.assertNotIn('LegacyApplicationProgramsFolder',text)
            self.assertIn('LegacyRelaunchFolder',text)
            self.assertIn('OpenWithProgids',text)
            self.assertIn('Name="M4B Editor"',text)
            self.assertIn('Name="m4b-Craft"',text)
    def test_compiled_msi_wizard(self):
        wixl = os.environ.get('WIXL', 'wixl')
        msibuild = os.environ.get('MSIBUILD', 'msibuild')
        msiinfo = os.environ.get('MSIINFO', 'msiinfo')
        if not all(shutil.which(tool) for tool in [wixl, msibuild, msiinfo]):
            self.skipTest('wixl and msitools required for compiled MSI tests')
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d)
            payload = root/'payload'
            payload.mkdir()
            (payload/'m4b-editor.exe').write_bytes(b'fixture')
            icon = root/release.SHORTCUT_ICON
            icon.write_bytes(b'fixture')
            wxs = root/'app.wxs'
            bridge = root/'bridge.exe'; bridge.write_bytes(b'fixture')
            release.wix(payload, wxs, icon, bridge)
            baseline = root/'baseline.msi'
            subprocess.run([wixl, '-a', 'x64', '-o', str(baseline), str(wxs)], check=True)
            msi = root/'wizard.msi'
            release.build_msi(wxs, msi, wixl, msibuild)
            inspect_wizard(msi)
            for table in ['InstallExecuteSequence', 'Upgrade', 'Directory', 'Component',
                          'File', 'Registry', 'RemoveFile', 'Shortcut']:
                self.assertCountEqual(msi_rows(msi, table), msi_rows(baseline, table), table)
            original_ui = msi_rows(baseline, 'InstallUISequence')
            patched_ui = [row for row in msi_rows(msi, 'InstallUISequence')
                          if not row['Action'].startswith('Wizard')]
            self.assertCountEqual(patched_ui, original_ui)

    def test_signing_binds_payload_and_rejects_tampering(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            path = Path(d)
            repo = path/'repo'
            (repo/'scripts').mkdir(parents=True)
            (repo/'packaging').mkdir()
            shutil.copy2(ROOT/'scripts/sign-update.py',repo/'scripts/sign-update.py')
            key = path/'key.pem'
            subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',str(key)],check=True)
            public = subprocess.check_output(['openssl','pkey','-in',str(key),'-pubout','-outform','DER'])
            (repo/'packaging/update-public-key.hex').write_text(public[-32:].hex())
            for platform, extension in [('windows-x86_64','msi'),('macos-aarch64','dmg')]:
                with self.subTest(platform=platform):
                    installer = path/f'M4B-Editor-0.4.0-{platform}.{extension}'
                    installer.write_bytes(b'test fixture')
                    output = path/f'update-v2-{platform}.json'
                    command = ['python3',str(repo/'scripts/sign-update.py'),'--key',str(key),
                        '--installer',str(installer),'--platform',platform,'--version','0.4.0',
                        '--out',str(output),'--scratch',str(path)]
                    subprocess.run(command,check=True,capture_output=True)
                    envelope = json.loads(output.read_text())
                    self.assertEqual(set(envelope),{'payload','signature'})
                    manifest = json.loads(envelope['payload'])
                    self.assertEqual(set(manifest),{'version','platform','filename','sha256','size','url'})
                    self.assertEqual(manifest['platform'],platform)
                    self.assertEqual(manifest['size'],installer.stat().st_size)
                    self.assertEqual(manifest['sha256'],sha256(installer))
                    self.assertEqual(manifest['filename'],installer.name)
                    self.assertEqual(manifest['url'],f'https://github.com/aruel-perso/m4b-editor/releases/download/v0.4.0/{installer.name}')
                    payload = path/'payload'; payload.write_bytes(envelope['payload'].encode('utf-8'))
                    signature = path/'signature'; signature.write_bytes(bytes.fromhex(envelope['signature']))
                    pub = path/'pub.der'; pub.write_bytes(public)
                    verify = ['openssl','pkeyutl','-verify','-rawin','-pubin','-keyform','DER',
                        '-inkey',str(pub),'-in',str(payload),'-sigfile',str(signature)]
                    self.assertEqual(subprocess.run(verify,capture_output=True).returncode,0)
                    legacy_installer = installer.with_name(installer.name.replace('M4B-Editor','m4b-Craft'))
                    shutil.copy2(installer, legacy_installer)
                    canonical_alias = command.copy()
                    canonical_alias[canonical_alias.index('--installer')+1] = str(legacy_installer)
                    self.assertNotEqual(subprocess.run(canonical_alias,capture_output=True).returncode,0)
                    legacy_command = command.copy() + ['--legacy']
                    legacy_command[legacy_command.index('--installer')+1] = str(legacy_installer)
                    legacy_output = path/f'update-{platform}.json'
                    legacy_command[legacy_command.index('--out')+1] = str(legacy_output)
                    subprocess.run(legacy_command,check=True,capture_output=True)
                    legacy_envelope = json.loads(legacy_output.read_text())
                    legacy_manifest = json.loads(legacy_envelope['payload'])
                    self.assertEqual(legacy_manifest['url'],f'https://github.com/aruel-perso/m4b-craft/releases/download/v0.4.0/{legacy_installer.name}')
                    payload.write_bytes(legacy_envelope['payload'].encode())
                    signature.write_bytes(bytes.fromhex(legacy_envelope['signature']))
                    self.assertEqual(subprocess.run(verify,capture_output=True).returncode,0)
                    payload.write_bytes(payload.read_bytes()+b' ')
                    self.assertNotEqual(subprocess.run(verify,capture_output=True).returncode,0)
                    wrong = command.copy(); wrong[wrong.index('--platform')+1] = 'macos-aarch64' if platform == 'windows-x86_64' else 'windows-x86_64'
                    result = subprocess.run(wrong,capture_output=True)
                    self.assertNotEqual(result.returncode,0)
                    self.assertIn(b'Expected',result.stderr)
                    if platform == 'windows-x86_64':
                        legacy = command.copy(); legacy[legacy.index('--installer')] = '--msi'
                        self.assertEqual(subprocess.run(legacy,capture_output=True).returncode,0)
                    for invalid in ['0.3.0-alpha.01','v0.3.0','0.3.0+build']:
                        bad = command.copy(); bad[bad.index('--version')+1] = invalid
                        self.assertNotEqual(subprocess.run(bad,capture_output=True).returncode,0)
    def test_signed_exact_allowlists_for_both_channels(self):
        spec = importlib.util.spec_from_file_location('assets',ROOT/'scripts/verify-release-assets.py')
        assets = importlib.util.module_from_spec(spec); spec.loader.exec_module(assets)
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d); repo = root/'repo'
            (repo/'scripts').mkdir(parents=True); (repo/'packaging').mkdir()
            shutil.copy2(ROOT/'scripts/sign-update.py',repo/'scripts/sign-update.py')
            key = root/'key.pem'
            subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',str(key)],check=True)
            public = subprocess.check_output(['openssl','pkey','-in',str(key),'-pubout','-outform','DER'])
            (repo/'packaging/update-public-key.hex').write_text(public[-32:].hex())
            for legacy in [False,True]:
                out = root/('legacy' if legacy else 'canonical'); out.mkdir()
                prefix = 'm4b-Craft' if legacy else 'M4B-Editor'
                platforms = [('windows-x86_64','msi'),('macos-aarch64','dmg')]
                if not legacy:
                    platforms.append(('linux-x86_64','deb'))
                for platform, ext in platforms:
                    base = f'{prefix}-0.4.0-{platform}'
                    suffixes = ['-media-sources.zip','-licenses.zip'] if legacy else ['.zip']
                    for suffix in ['.'+ext] + suffixes:
                        file = out/(base+suffix); file.write_bytes(b'offline fixture')
                        file.with_name(file.name+'.sha256').write_text(sha256(file)+'  '+file.name+'\n')
                    manifest = out/(f'update-{platform}.json' if legacy else f'update-v2-{platform}.json')
                    command = ['python3',str(repo/'scripts/sign-update.py'),'--key',str(key),'--version','0.4.0',
                               '--platform',platform,'--installer',str(out/(base+'.'+ext)),
                               '--out',str(manifest),'--scratch',str(root)] + (['--legacy'] if legacy else [])
                    subprocess.run(command,check=True,capture_output=True)
                with patch.object(assets,'ROOT',repo):
                    assets.verify(out,'0.4.0',legacy=legacy,signed=True)
                    wrong = next(out.glob('update-*.json'))
                    envelope = json.loads(wrong.read_text()); payload = json.loads(envelope['payload'])
                    payload['url'] = payload['url'].replace('/releases/', '/redirect/releases/')
                    envelope['payload'] = json.dumps(payload); wrong.write_text(json.dumps(envelope))
                    with self.assertRaisesRegex(ValueError,'origin/content'):
                        assets.verify(out,'0.4.0',legacy=legacy,signed=True)

    def test_prepare_legacy_preserves_windows_bytes_and_requires_legacy_dmg(self):
        spec = importlib.util.spec_from_file_location('legacy',ROOT/'scripts/prepare-legacy-release.py')
        legacy = importlib.util.module_from_spec(spec); spec.loader.exec_module(legacy)
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d); canonical = root/'canonical'; canonical.mkdir()
            windows = canonical/'M4B-Editor-0.4.0-windows-x86_64.msi'
            windows.write_bytes(b'exact offline Windows installation fixture')
            for platform in ['windows-x86_64','macos-aarch64']:
                for suffix in ['media-sources','licenses']:
                    (canonical/f'M4B-Editor-0.4.0-{platform}-{suffix}.zip').write_bytes(b'sidecar fixture')
            dmg = root/'m4b-Craft-0.4.0-macos-aarch64.dmg'; dmg.write_bytes(b'legacy image fixture')
            out = root/'output'
            legacy.prepare(canonical,dmg,out,'0.4.0')
            self.assertEqual((out/'m4b-Craft-0.4.0-windows-x86_64.msi').read_bytes(),windows.read_bytes())
            self.assertEqual((out/dmg.name).read_bytes(),dmg.read_bytes())
            with self.assertRaisesRegex(ValueError,'empty'):
                legacy.prepare(canonical,dmg,out,'0.4.0')
            canonical_dmg = root/'M4B-Editor-0.4.0-macos-aarch64.dmg'
            canonical_dmg.write_bytes(b'canonical layout is incompatible')
            with self.assertRaisesRegex(ValueError,'separately built'):
                legacy.prepare(canonical,canonical_dmg,root/'bad','0.4.0')

    def test_legacy_sources_are_rebuilt_from_current_portable_payload(self):
        from release_common import MEDIA_LICENSES
        spec = importlib.util.spec_from_file_location('legacy', ROOT/'scripts/prepare-legacy-release.py')
        legacy = importlib.util.module_from_spec(spec); spec.loader.exec_module(legacy)
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d)
            canonical = root/'canonical'; canonical.mkdir()
            compat = root/'compat'; compat.mkdir()
            name = 'M4B-Editor-0.4.2-windows-x86_64'
            (canonical/f'{name}.msi').write_bytes(b'windows fixture')
            with zipfile.ZipFile(canonical/f'{name}.zip', 'w') as portable:
                for item in (*SOURCES, *MEDIA_LICENSES):
                    portable.writestr('source/third-party/ffmpeg/'+item, b'media fixture')
                for item in ('LICENSE', 'THIRD-PARTY-NOTICES.txt', 'licenses/test/LICENSE'):
                    portable.writestr(item, b'license fixture')
            dmg = compat/'m4b-Craft-0.4.2-macos-aarch64.dmg'
            dmg.write_bytes(b'mac fixture')
            for suffix in ('media-sources', 'licenses'):
                dmg.with_name(f'{dmg.stem}-{suffix}.zip').write_bytes(b'mac sidecar fixture')
            output = root/'output'
            legacy.prepare(canonical, dmg, output, '0.4.2')
            with zipfile.ZipFile(output/'m4b-Craft-0.4.2-windows-x86_64-media-sources.zip') as archive:
                self.assertEqual(set(archive.namelist()), set(SOURCES))
            with zipfile.ZipFile(output/'m4b-Craft-0.4.2-windows-x86_64-licenses.zip') as archive:
                self.assertIn('licenses/test/LICENSE', archive.namelist())
                self.assertIn('media/COPYING.LGPLv2.1', archive.namelist())

    def test_source_pins_match_fetch_and_platform_recipes(self):
        for name, digest in SOURCES.items():
            for script in ['fetch-media-sources.sh','build-ffmpeg-windows.sh','build-ffmpeg-unix.sh']:
                text = (ROOT/'scripts'/script).read_text()
                self.assertIn(name,text)
                self.assertIn(digest,text)

    def test_license_sidecar_contains_notices_and_all_media_licenses(self):
        from release_common import license_sidecar
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d); payload = root/'payload'; payload.mkdir()
            media = root/'media'; media.mkdir()
            (payload/'licenses/crate').mkdir(parents=True)
            (payload/'licenses/crate/LICENSE').write_text('crate license fixture')
            (payload/'licenses/crate/LICENSES').mkdir()
            (payload/'licenses/crate/LICENSES/NOTICE.txt').write_text('nested upstream license')
            for name in ['LICENSE','THIRD-PARTY-NOTICES.txt']:
                (payload/name).write_text(name)
            for name in ['COPYING.LGPLv2.1','LICENSE.md','LICENSE.zlib','COPYING.LAME']:
                (media/name).write_text(name)
            output = root/'licenses.zip'; license_sidecar(payload,media,output)
            with zipfile.ZipFile(output) as archive:
                self.assertIsNone(archive.testzip())
                self.assertEqual(archive.read('licenses/crate/LICENSE'),b'crate license fixture')
                self.assertEqual(archive.read('media/COPYING.LAME'),b'COPYING.LAME')
                self.assertEqual(archive.read('licenses/crate/LICENSES/NOTICE.txt'), b'nested upstream license')
                self.assertEqual(len(archive.namelist()),8)

    def test_archive_contains_real_source_bytes(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            root = Path(d); source = root/'source'
            release.copy_source(source)
            output = root/'source.zip'; release.archive(source,output)
            with zipfile.ZipFile(output) as archive:
                self.assertIsNone(archive.testzip())
                self.assertEqual(archive.read('Cargo.lock'),(ROOT/'Cargo.lock').read_bytes())
                self.assertEqual(archive.read('scripts/ffmpeg-options.sh'),(ROOT/'scripts/ffmpeg-options.sh').read_bytes())
                self.assertNotIn('BRIEF.md',archive.namelist())

    def test_signing_refuses_wrong_external_key(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            path = Path(d)
            key = path/'key.pem'
            subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',str(key)],check=True)
            msi = path/'M4B-Editor-0.4.0-windows-x86_64.msi'; msi.write_bytes(b'test fixture')
            output = path/'update-v2-windows-x86_64.json'
            result = subprocess.run(['python3',str(ROOT/'scripts/sign-update.py'),'--key',str(key),
                '--msi',str(msi),'--version','0.4.0','--out',str(output),'--scratch',str(path)],capture_output=True)
            self.assertNotEqual(result.returncode,0)
            self.assertIn(b'does not match pinned public key',result.stderr)
            self.assertFalse(output.exists())

    def test_media_cache_requires_exact_records_and_pinned_sources(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            media = Path(d)
            names = ['ffmpeg','ffprobe', *MEDIA_BUILD_FILES, *SOURCES]
            for name in names: (media/name).write_bytes(b'fixture')
            def manifest():
                (media/'SHA256SUMS').write_text(''.join(sha256(media/name)+'  '+name+'\n' for name in names))
            manifest()
            with self.assertRaisesRegex(ValueError,'Unpinned source'): verify_media(media,'macos-aarch64')
            with patch.dict(SOURCES,{name:sha256(media/name) for name in SOURCES}):
                verify_media(media,'macos-aarch64')
                legacy = LEGACY_ARCHIVER_SOURCES | LEGACY_ARCHIVER_MEMBERS['macos-aarch64']
                for name in legacy: (media/name).write_bytes(b'legacy fixture')
                names.extend(legacy)
                manifest()
                with patch.dict(LEGACY_ARCHIVER_SOURCES, {name:sha256(media/name) for name in LEGACY_ARCHIVER_SOURCES}), patch.dict(LEGACY_ARCHIVER_MEMBERS['macos-aarch64'], {name:sha256(media/name) for name in LEGACY_ARCHIVER_MEMBERS['macos-aarch64']}):
                    verify_media(media,'macos-aarch64')
                for name in legacy:
                    names.remove(name)
                    (media/name).unlink()
                manifest()
                (media/'ffmpeg').write_bytes(b'corrupted')
                with self.assertRaisesRegex(ValueError,'checksum mismatch'): verify_media(media,'macos-aarch64')
                manifest()
                names.remove('ffprobe'); manifest()
                with self.assertRaisesRegex(ValueError,'cache contents'): verify_media(media,'macos-aarch64')

    def test_macho_rejects_external_dylibs_and_wrong_architecture(self):
        spec = importlib.util.spec_from_file_location('macho',ROOT/'scripts/verify-macos.py')
        macho = importlib.util.module_from_spec(spec); spec.loader.exec_module(macho)
        for arch, dependency, accepted in [
            ('arm64','/usr/lib/libSystem.B.dylib',True),
            ('arm64','/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit',True),
            ('x86_64','/usr/lib/libSystem.B.dylib',False),
            ('arm64 x86_64','/usr/lib/libSystem.B.dylib',False),
            ('arm64','/opt/homebrew/lib/libz.1.dylib',False),
            ('arm64','@rpath/libz.1.dylib',False),
            ('arm64','/usr/local/lib/libz.1.dylib',False)]:
            with self.subTest(arch=arch,dependency=dependency):
                with patch.object(macho.subprocess,'check_output',side_effect=[arch+'\n',f'fixture:\n\t{dependency} (compatibility version 1.0.0, current version 1.0.0)\n']):
                    if accepted: macho.verify(Path('fixture'))
                    else:
                        with self.assertRaises(ValueError): macho.verify(Path('fixture'))

    def test_flat_assets_fail_on_missing_nested_or_changed_files(self):
        spec = importlib.util.spec_from_file_location('assets',ROOT/'scripts/verify-release-assets.py')
        assets = importlib.util.module_from_spec(spec); spec.loader.exec_module(assets)
        with tempfile.TemporaryDirectory(dir=SCRATCH) as d:
            directory = Path(d)
            for platform, installer in [('windows-x86_64','msi'),('macos-aarch64','dmg'),('linux-x86_64','deb')]:
                for suffix in ['.'+installer, '.zip']:
                    file = directory/f'M4B-Editor-0.4.0-{platform}{suffix}'
                    file.write_bytes(b'fixture')
                    file.with_name(file.name+'.sha256').write_text(sha256(file)+'  '+file.name+'\n')
            assets.verify(directory,'0.4.0')
            extra = directory/'nested'; extra.mkdir()
            with self.assertRaises(ValueError): assets.verify(directory,'0.4.0')
            extra.rmdir()
            next(directory.glob('*.dmg')).write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError,'Checksum mismatch'): assets.verify(directory,'0.4.0')

def msi_rows(msi, table):
    tool = os.environ.get('MSIINFO', 'msiinfo')
    lines = subprocess.check_output([tool, 'export', str(msi), table], encoding='utf-8').splitlines()
    columns = lines[0].split('\t')
    return [dict(zip(columns, line.split('\t'), strict=True)) for line in lines[3:]]


def inspect_wizard(msi):
    sequence = {row['Action']: row for row in msi_rows(msi, 'InstallUISequence')}
    order = {action: int(row['Sequence']) for action, row in sequence.items()}
    assert order['CostFinalize'] < order['WizardWelcome'] < order['WizardProgress'] < order['ExecuteAction']
    assert not sequence['ExecuteAction']['Condition']
    for action, expected in [('WizardComplete', -1), ('WizardCancelled', -2), ('WizardError', -3)]:
        assert order[action] == expected
    for action in ['WizardWelcome', 'WizardProgress', 'WizardComplete', 'WizardCancelled', 'WizardError']:
        assert sequence[action]['Condition'] == (
            'UILevel = 5 AND NOT Installed AND NOT REMOVE AND NOT UPGRADINGPRODUCTCODE AND NOT RESUME')
    execute = {row['Action']: int(row['Sequence']) for row in msi_rows(msi, 'InstallExecuteSequence')}
    assert all(not action.startswith('Wizard') for action in execute)
    assert execute['InstallInitialize'] < execute['RemoveExistingProducts'] < execute['InstallFiles']
    assert execute['RemoveShortcuts'] < execute['CreateShortcuts'] < execute['InstallFinalize']

    properties = {row['Property']: row['Value'] for row in msi_rows(msi, 'Property')}
    assert not properties.get('ALLUSERS')
    assert properties['DefaultUIFont'] == 'WizardBody'
    directories = {row['Directory']: row for row in msi_rows(msi, 'Directory')}
    assert directories['INSTALLDIR']['Directory_Parent'] == 'Programs'
    assert directories['INSTALLDIR']['DefaultDir'].split('|')[-1] == 'M4B Editor'
    assert directories['LegacyRelaunchFolder']['DefaultDir'].split('|')[-1] == 'm4b-Craft'
    assert directories['Programs']['Directory_Parent'] == 'LocalAppDataFolder'

    dialogs = {row['Dialog']: row for row in msi_rows(msi, 'Dialog')}
    assert set(dialogs) == {
        'WizardWelcome', 'WizardReady', 'WizardProgress', 'WizardConfirmCancel',
        'WizardComplete', 'WizardCancelled', 'WizardError',
    }
    assert not int(dialogs['WizardProgress']['Attributes']) & 2
    assert dialogs['WizardReady']['Control_Default'] == 'Install'
    controls = {(row['Dialog_'], row['Control']): row for row in msi_rows(msi, 'Control')}
    assert controls['WizardReady', 'Destination']['Text'] == '[INSTALLDIR]'
    assert 'Créez' in controls['WizardWelcome', 'Message']['Text']
    icon = controls['WizardWelcome', 'AppIcon']
    assert (icon['Type'], icon['Text']) == ('Icon', 'WizardIcon')
    tool = os.environ.get('MSIINFO', 'msiinfo')
    assert subprocess.check_output([tool, 'extract', str(msi), 'Binary.WizardIcon']) == (
        ROOT/'packaging/icons/app.ico').read_bytes()

    control_events = msi_rows(msi, 'ControlEvent')
    events = {(row['Dialog_'], row['Control_']): (row['Event'], row['Argument'], row['Condition'])
              for row in control_events if row['Event'] != 'DoAction'}
    launch = controls['WizardComplete', 'Launch']
    assert (launch['Type'], launch['Property'], launch['Text']) == (
        'CheckBox', 'WizardLaunchEditor', 'Lancer M4B Editor')
    assert 'WizardLaunchEditor' not in properties
    assert not dialogs['WizardComplete']['Control_Cancel']
    assert msi_rows(msi, 'CheckBox') == [{'Property': 'WizardLaunchEditor', 'Value': '1'}]
    assert [row for row in control_events if row['Event'] == 'DoAction'] == [{
        'Dialog_': 'WizardComplete', 'Control_': 'Finish', 'Event': 'DoAction',
        'Argument': 'WizardStartEditor', 'Condition': sequence['WizardComplete']['Condition']
        + ' AND WizardLaunchEditor = "1"', 'Ordering': '1',
    }]
    custom = [row for row in msi_rows(msi, 'CustomAction') if row['Action'] == 'WizardStartEditor']
    assert len(custom) == 1 and custom[0]['Type'] == '210'
    assert custom[0]['Source'] == 'EditorExecutable'
    assert 'WizardStartEditor' not in sequence and 'WizardStartEditor' not in execute
    assert all(row['Control'] != 'Launch' or row['Dialog_'] == 'WizardComplete'
               for row in controls.values())
    components = {row['Component']: row for row in msi_rows(msi, 'Component')}
    assert components['LegacyRelaunchCraft']['Directory_'] == 'LegacyRelaunchFolder'
    assert components['LegacyRelaunchEditor']['Directory_'] == 'LegacyRelaunchFolder'
    assert {row['Shortcut'] for row in msi_rows(msi, 'Shortcut')} == {'StartMenuShortcut'}
    registry = msi_rows(msi, 'Registry')
    assert not any('m4b-Craft' in row['Key'] for row in registry)
    assert any(row['Key'].endswith('M4BEditor.Project\\shell\\open\\command')
               and row['Value'] == '"[#EditorExecutable]" "%1"' for row in registry)
    assert events['WizardWelcome', 'Next'] == ('NewDialog', 'WizardReady', '1')
    assert events['WizardReady', 'Back'] == ('NewDialog', 'WizardWelcome', '1')
    assert events['WizardReady', 'Install'] == ('EndDialog', 'Return', '1')
    for name in ['WizardWelcome', 'WizardReady', 'WizardProgress']:
        assert dialogs[name]['Control_Cancel'] == 'Cancel'
        assert events[name, 'Cancel'] == ('SpawnDialog', 'WizardConfirmCancel', '1')
    assert events['WizardConfirmCancel', 'Continue'] == ('EndDialog', 'Return', '1')
    assert events['WizardConfirmCancel', 'Abort'] == ('EndDialog', 'Exit', '1')
    for name in ['WizardComplete', 'WizardCancelled', 'WizardError']:
        assert events[name, 'Finish'] == ('EndDialog', 'Return', '1')
    assert msi_rows(msi, 'EventMapping') == [
        {'Dialog_': 'WizardProgress', 'Control_': 'Progress', 'Event': 'SetProgress', 'Attribute': 'Progress'}]


def inspect(msi, scratch):
    inspect_wizard(msi)
    tool = os.environ.get('MSIINFO','msiinfo')
    properties = subprocess.check_output([tool,'export',str(msi),'Property']).decode()
    for value in [release.UPGRADE_CODE.upper(), release.MSI_VERSION]:
        assert value.lower() in properties.lower(),value
    property_rows = dict(row.split('\t', 1) for row in properties.splitlines()[3:] if '\t' in row)
    assert property_rows.get('ProductName') == 'M4B Editor', property_rows.get('ProductName')
    shortcuts = subprocess.check_output([tool,'export',str(msi),'Shortcut'], text=True).splitlines()
    columns = shortcuts[0].split('\t')
    rows = {row['Shortcut']: row for row in
            (dict(zip(columns, line.split('\t'))) for line in shortcuts[3:])}
    for name in ['StartMenuShortcut']:
        assert rows[name]['Icon_'] == release.SHORTCUT_ICON
        assert rows[name]['IconIndex'] == '0'
    icon_stream = f'Icon.{release.SHORTCUT_ICON}'
    with tempfile.TemporaryDirectory(dir=scratch) as d:
        icon = Path(d)/release.SHORTCUT_ICON
        icon.write_bytes(subprocess.check_output([tool,'extract',str(msi),icon_stream]))
        verify_pe_icon(icon)
    files = subprocess.check_output([tool,'export',str(msi),'File']).decode()
    for name in ['m4b-editor.exe','m4b-editor-cli.exe','ffmpeg.exe','ffprobe.exe','Cargo.lock','LICENSE']:
        assert name in files,name
    assert '7z.exe' not in files and '7z.dll' not in files
    assert '\nALLUSERS\t1' not in properties
    directories = subprocess.check_output([tool,'export',str(msi),'Directory']).decode()
    assert 'LocalAppDataFolder' in directories
    sequence = subprocess.check_output([tool,'export',str(msi),'InstallExecuteSequence']).decode()
    actions = {row.split('\t')[0]: row.split('\t')[-1].strip() for row in sequence.splitlines()[3:]}
    assert int(actions['RemoveExistingProducts']) > int(actions['InstallInitialize'])
    upgrade = subprocess.check_output([tool,'export',str(msi),'Upgrade']).decode()
    assert release.UPGRADE_CODE.lower() in upgrade.lower()
    with zipfile.ZipFile(msi.with_suffix('.zip')) as archive:
        assert archive.testzip() is None
        for name in ['m4b-editor.exe','m4b-editor-cli.exe','ffmpeg.exe','ffprobe.exe','source/crates/desktop/src/updater.rs','source/Cargo.lock']:
            assert name in archive.namelist(),name
        assert not any(name.lower().endswith(('.pem','.key','7z.exe','7z.dll','license.7zip')) for name in archive.namelist())
        for name, digest in SOURCES.items():
            embedded = archive.read('source/third-party/ffmpeg/'+name)
            assert hashlib.sha256(embedded).hexdigest() == digest
    with tempfile.TemporaryDirectory(dir=scratch) as d:
        subprocess.run([os.environ.get('MSIEXTRACT','msiextract'),'-C',d,str(msi)],check=True,stdout=subprocess.DEVNULL)
        for path in Path(d).rglob('m4b-editor.exe'):
            if path.parent.name == 'm4b-Craft':
                continue
            verify_pe_icon(path)
            assert (path.parent/'source/crates/desktop/src/updater.rs').exists()
            assert (path.parent/'ffprobe.exe').exists()
            from release_common import verify_source_archives
            verify_source_archives(path.parent/'source/third-party/ffmpeg')
            break
        else: raise AssertionError('Missing extracted application')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('--scratch',required=True,type=Path)
    parser.add_argument('--msi',type=Path); args = parser.parse_args()
    SCRATCH = args.scratch; SCRATCH.mkdir(parents=True,exist_ok=True)
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(ReleaseTests))
    if not result.wasSuccessful(): raise SystemExit(1)
    if args.msi:
        inspect(args.msi.resolve(),SCRATCH)
        print('PASS MSI tables/extraction, portable ZIP contents/CRC, embedded source hashes, key exclusion')
