#!/usr/bin/env python3
"""Run actual Windows Rust tests with the bundled media tools and staged fixtures."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('cargo_json', type=Path)
parser.add_argument('--media', type=Path)
parser.add_argument('--fixtures', type=Path)
parser.add_argument('--test-filter', help='Optional Rust test-name filter for focused verification')
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
media = (args.media or root/'vendor/windows-media').resolve()
fixtures = (args.fixtures or root/'tests/fixtures').resolve()
scratch = Path(os.environ.get('TMPDIR', str(Path.home()/'.hermes/cache/scratch')))
scratch.mkdir(parents=True, exist_ok=True)
executables = []
for line in args.cargo_json.read_text().splitlines():
    message = json.loads(line)
    if message.get('reason') == 'compiler-artifact' and message.get('profile', {}).get('test') and message.get('executable'):
        executable = Path(message['executable']).resolve()
        if executable not in executables: executables.append(executable)
assert executables, 'No test artifacts found'
wine = shutil.which('wine64') or '/usr/lib/wine/wine64'
server = shutil.which('wineserver') or '/usr/lib/wine/wineserver64'
with tempfile.TemporaryDirectory(prefix='m4b-editor-windows-tests-', dir=scratch) as temporary:
    temp = Path(temporary)
    for name in ['ffmpeg.exe','ffprobe.exe']:
        shutil.copy2(media/name,temp/name)
    staged_fixtures = temp/'source/tests/fixtures'
    shutil.copytree(fixtures, staged_fixtures)
    # The frozen corpus also references the pre-existing MP3 regression outside
    # tests/fixtures. Preserve manifest-relative layout rather than omitting it.
    for entry in json.loads((fixtures/'manifest.json').read_text())['files']:
        source = (fixtures/entry['path']).resolve()
        source.relative_to(fixtures.parents[1])
        target = (staged_fixtures/entry['path']).resolve()
        target.relative_to(temp/'source')
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
    def windows(path): return 'Z:'+str(path).replace('/','\\')
    runtime = temp/'runtime'; runtime.mkdir(mode=0o700)
    env = dict(os.environ,WINEDEBUG='-all',WINEPREFIX=str(temp/'prefix'),
               XDG_RUNTIME_DIR=str(runtime),WINEPATH=windows(temp),
               M4B_EDITOR_TEST_FIXTURES=windows(staged_fixtures))
    try:
        for executable in executables:
            copied = temp/executable.name
            shutil.copy2(executable,copied)
            print('RUN Windows:',executable.name,flush=True)
            command = [wine,str(copied),'--nocapture','--test-threads=1']
            if args.test_filter: command.append(args.test_filter)
            subprocess.run(command, cwd=temp,env=env,timeout=180,check=True)
        print(f'PASS {len(executables)} Windows Rust test executables with bundled FFmpeg/ffprobe and staged fixtures',flush=True)
    finally:
        subprocess.run([server,'-k'],env=env,timeout=20,check=False)
