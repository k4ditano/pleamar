"""Exercise shared rule timers across two native copies on one chosen monitor.

Both copies stay on --screen. No audio services, physical input or focus requests.
"""
from pathlib import Path
import argparse
import ctypes
import os
import subprocess
import tempfile
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary', type=Path, required=True)
p.add_argument('--screen', required=True)
args = p.parse_args()
binary = args.binary.resolve()
root = Path(__file__).resolve().parents[1]
user32 = ctypes.WinDLL('user32')
user32.GetForegroundWindow.restype = ctypes.c_void_p
foreground = user32.GetForegroundWindow()
with tempfile.TemporaryDirectory(prefix='pleamar rule handoff ñ ') as tmp:
    scene = Path(tmp) / 'handoff.plm'
    scene.write_bytes((root / 'tests/still-monitor.plm').read_bytes())
    env = dict(os.environ, APPDATA=tmp, PLEAMAR_SOCKET_DIR='rule-handoff-' + str(os.getpid()),
        PLEAMAR_NO_RELAUNCH='1')
    with (Path(tmp) / 'render.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen', args.screen + ',' + args.screen,
            '--no-hud', '--stall', '0', '--seconds', '30'], env=env,
            stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
        def ask(command):
            return subprocess.check_output([str(binary), '--say', scene.stem, command], env=env,
                text=True, encoding='utf-8', stderr=subprocess.DEVNULL, timeout=3,
                creationflags=subprocess.CREATE_NO_WINDOW).strip()
        def wait_meter(value, timeout=4):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline and process.poll() is None:
                try:
                    if abs(float(ask('get meter')) - value) < .01: return
                except (subprocess.SubprocessError, ValueError): pass
                time.sleep(.03)
            raise AssertionError(f'meter did not settle at {value}; current={ask("get meter")}')
        try:
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                if 'first frame' in (Path(tmp) / 'render.log').read_text(encoding='utf-8'): break
                time.sleep(.1)
            else: raise AssertionError('No native frame')
            wait_meter(0)
            ask('fact volume 1')
            wait_meter(1)
            ask('fact home 1')
            wait_meter(0)
            # A second burst on the new copy must renew the full deadline.
            ask('fact volume 0.8')
            wait_meter(1)
            ask('fact volume 0.6')
            time.sleep(.65)
            assert float(ask('get meter')) > .99
            wait_meter(0)
            ask('fact home 0')
            time.sleep(.4)
            assert float(ask('get meter')) < .01, 'stale change replayed on the original copy'
            assert user32.GetForegroundWindow() == foreground, 'foreground changed'
            print('PASS: native copy handoff, volume burst deadline, no stale replay; foreground unchanged')
        finally:
            if process.poll() is None:
                try: ask('quit'); process.wait(timeout=10)
                except (subprocess.SubprocessError, OSError): process.terminate(); process.wait(timeout=10)
