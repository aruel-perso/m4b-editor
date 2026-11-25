#!/usr/bin/env python3
"""Linux startup smoke. Run: xvfb-run -a python3 scripts/smoke-desktop.py
Requires xdotool; no user state or media is modified. Not an interactive UI test.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/m4b-editor').resolve()
with tempfile.TemporaryDirectory(prefix='m4b-ui-smoke-') as state:
    env = os.environ.copy()
    env.update(XDG_STATE_HOME=state, XDG_CONFIG_HOME=state,
               XDG_CACHE_HOME=state, XDG_RUNTIME_DIR=state, LIBGL_ALWAYS_SOFTWARE='1')
    with tempfile.TemporaryFile() as log:
        app = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
        try:
            window = None
            for _ in range(100):
                if app.poll() is not None:
                    log.seek(0)
                    raise AssertionError(f'Desktop exited {app.returncode}: {log.read().decode(errors="replace")}')
                result = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', '^M4B Editor [0-9]'],
                                        env=env, capture_output=True, text=True, timeout=2)
                if result.returncode == 0:
                    window = result.stdout.strip()
                    break
                time.sleep(.1)
            assert window, 'No visible M4B Editor window within 10 seconds'
            time.sleep(2)
            log_file = Path(state)/'m4b-editor/log/diagnostic-0.log'
            assert 'application_start version=' in log_file.read_text()
            assert app.poll() is None, 'Desktop exited after window creation'
            print('PASS: native desktop starts, creates a visible M4B Editor window, writes its startup log and remains alive under Xvfb/software OpenGL')
            print('This smoke intentionally terminates the app; it does not test editor interaction or normal close.')
        finally:
            if app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait(timeout=5)
