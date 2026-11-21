#!/usr/bin/env python3
"""Native DMG mount/install/CLI media test. Does not launch or test the GUI."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import struct
import subprocess
import tempfile
import wave
import zipfile
from release_common import ROOT, SOURCES, VERSION

spec = importlib.util.spec_from_file_location('verify',Path(__file__).with_name('verify-macos.py'))
verify = importlib.util.module_from_spec(spec); spec.loader.exec_module(verify)
def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args],check=True,**kwargs)
def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--legacy',action='store_true')
    p.add_argument('--dmg',required=True,type=Path); p.add_argument('--scratch',required=True,type=Path)
    a = p.parse_args()
    assert os.uname().machine == 'arm64' and os.uname().sysname == 'Darwin'
    dmg = a.dmg.resolve()
    sidecar = dmg.with_name(dmg.stem+'-media-sources.zip')
    with zipfile.ZipFile(sidecar) as archive:
        assert set(archive.namelist()) == set(SOURCES)
        assert archive.testzip() is None
        for name,digest in SOURCES.items(): assert hashlib.sha256(archive.read(name)).hexdigest() == digest
    with tempfile.TemporaryDirectory(prefix='m4b-dmg-test-',dir=a.scratch) as temporary:
        work = Path(temporary); mount = work/'mounted'; mount.mkdir()
        run('hdiutil','attach','-readonly','-nobrowse','-mountpoint',mount,dmg)
        try:
            assert (mount/'Applications').is_symlink()
            assert os.readlink(mount/'Applications') == '/Applications'
            source = mount/('m4b-Craft.app' if a.legacy else 'M4B Editor.app')
            info = plistlib.loads((source/'Contents/Info.plist').read_bytes())
            assert info['CFBundleExecutable'] == ('m4b-Craft' if a.legacy else 'm4b-editor')
            assert info['CFBundleShortVersionString'] == VERSION.split('-')[0]
            assert info['UTExportedTypeDeclarations'][0]['UTTypeTagSpecification']['public.filename-extension'] == ['m4bproj']
            resources = source/'Contents/Resources'
            assert info['CFBundleIconFile'] == 'app.icns'
            assert (resources/info['CFBundleIconFile']).read_bytes() == (ROOT/'packaging/icons/app.icns').read_bytes()
            for required in ['LICENSE','THIRD-PARTY-NOTICES.txt','MEDIA-SOURCES.txt',
                             'source/Cargo.toml','source/Cargo.lock','source/scripts/build-ffmpeg-macos.sh',
                             'source/third-party/ffmpeg/COPYING.LGPLv2.1',
                             'source/third-party/ffmpeg/LICENSE.zlib',
                             'source/third-party/ffmpeg/config.mak',
                             'source/third-party/ffmpeg/zlib-config.txt']:
                assert (resources/required).is_file(), required
            assert info['CFBundleDocumentTypes'][0]['LSItemContentTypes'] == ['org.m4bcraft.project']
            assert not any(f.suffix.lower() in ('.pem','.key') for f in source.rglob('*'))
            run('codesign','--verify','--deep','--strict',source)
            install = work/'Writable Applications'; install.mkdir()
            installed = install/source.name
            run('ditto',source,installed)
        finally:
            run('hdiutil','detach',mount)
        run('codesign','--verify','--deep','--strict',installed)
        binaries = installed/'Contents/MacOS'
        for name in ['m4b-editor','m4b-editor-cli','ffmpeg','ffprobe']: verify.verify(binaries/name)
        assert not (binaries/'7zz').exists()
        # A system-only PATH proves installed CLI finds its adjacent media tools.
        env = dict(os.environ,PATH='/usr/bin:/bin:/usr/sbin:/sbin')
        cli = binaries/'m4b-editor-cli'
        version = subprocess.check_output([str(cli),'--version'],text=True,env=env)
        assert VERSION in version
        for fixture in ['audio.zip', 'audio.rar']:
            archived_project = work/(fixture + '.m4bproj')
            run(cli,'new','--project',archived_project,ROOT/'tests/fixtures'/fixture,env=env)
            assert len(json.loads(archived_project.read_text())['tracks']) == 1

        track = work/'PCM é sample.wav'
        with wave.open(str(track),'wb') as output:
            output.setparams((1,2,44100,0,'NONE','not compressed'))
            output.writeframes(b''.join(struct.pack('<h', 8000 if (i//50)%2 else -8000) for i in range(44100)))
        project = work/'book.m4bproj'; book = work/'book.m4b'
        run(cli,'new','--project',project,'--title','Installed CLI smoke',track,env=env)
        for codec, extension in [('libmp3lame','mp3'),('flac','flac')]:
            encoded = work/f'codec-smoke.{extension}'
            run(binaries/'ffmpeg','-v','error','-nostdin','-i',track,'-c:a',codec,encoded,env=env)
            run(binaries/'ffmpeg','-v','error','-nostdin','-i',encoded,'-f','null','-',env=env)
            data = json.loads(subprocess.check_output([str(binaries/'ffprobe'),'-v','error','-show_streams','-of','json',str(encoded)],env=env))
            assert data['streams'][0]['codec_name'] == extension
        run(cli,'check',project,env=env)
        run(cli,'export',project,book,env=env)
        run(cli,'probe',book,env=env)
        probe = json.loads(subprocess.check_output([str(binaries/'ffprobe'),'-v','error','-show_streams','-show_chapters','-of','json',str(book)],env=env))
        assert probe['streams'][0]['codec_name'] == 'aac'
        assert probe['chapters']
        run(binaries/'ffmpeg','-v','error','-i',book,'-f','null','-',env=env)
        imported = work/'imported.m4bproj'
        run(cli,'open',book,'--project',imported,env=env)
        run(cli,'export',imported,work/'reexport.m4b',env=env)
        print('PASS mounted DMG, writable app copy, installed CLI PCM/AAC export/import/re-export; GUI untested')
if __name__ == '__main__': main()
