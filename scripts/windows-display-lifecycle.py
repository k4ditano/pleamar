"""Native GPU/display-lifecycle rehearsal with process-local output filtering.

Requires the ignored Rust test helper and a real Windows desktop. This changes
only which real monitor the test process observes, never Windows display state.
It validates runtime lifecycle, not physical hotplug or driver/DPI migration.
"""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import os
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True, help='native pleamar executable for IPC')
parser.add_argument('--harness', type=Path, required=True, help='compiled pleamar library test executable')
parser.add_argument('--output', type=Path, required=True, help='new directory for evidence and isolated state')
parser.add_argument('--interactive', action='store_true', help='wait for an actual panel click after reconnection')
args = parser.parse_args()
assert os.name == 'nt', 'Native Windows rehearsal'
root = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('windows_smoke', root / 'scripts/windows-smoke.py')
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
binary, harness, out = args.binary.resolve(), args.harness.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
scene = out / 'native-display-lifecycle.plm'
for suffix in ('.plm', '.luau'):
    scene.with_suffix(suffix).write_bytes((root / 'tests' / scene.with_suffix(suffix).name).read_bytes())
control = out / 'output-state.txt'

def connected(value):
    staging = control.with_suffix('.new')
    staging.write_text('connected' if value else 'absent', encoding='utf-8')
    staging.replace(control)

connected(False)
env = dict(os.environ, PLEAMAR_DISPLAY_TEST_SCENE=str(scene), PLEAMAR_DISPLAY_TEST_CONTROL=str(control),
    PLEAMAR_SOCKET_DIR=f'display-lifecycle-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1',
    PLEAMAR_TEST_WINDOWS='1', APPDATA=str(out / 'state'))
report = {'complete': False, 'phases': [], 'physical_input_requested': args.interactive,
    'harness_sha256': hashlib.sha256(harness.read_bytes()).hexdigest(),
    'ipc_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    'scene_sha256': hashlib.sha256(scene.read_bytes()).hexdigest()}
process = None

def ask(command):
    return subprocess.check_output([str(binary), '--say', scene.stem, command], env=env,
        encoding='utf-8', stderr=subprocess.DEVNULL, timeout=5, creationflags=subprocess.CREATE_NO_WINDOW).strip()

def until(check, label, seconds=45):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if process.poll() is not None: raise RuntimeError(f'{label}: native process exited {process.returncode}')
        try:
            if check(): return
        except (subprocess.SubprocessError, ValueError): pass
        time.sleep(.15)
    raise RuntimeError(label)

def phase(name):
    state = {key: ask('get ' + key) for key in ('screens.count', 'ticks', 'clicks', 'open')}
    state.update(name=name, native_hwnds=smoke.native_window_count(process.pid, visible_only=False))
    report['phases'].append(state)
    (out / 'report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(state), flush=True)
    return state

try:
    with (out / 'native.log').open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(harness), '--exact', 'platform::windows::input_tests::native_display_lifecycle_helper', '--ignored', '--nocapture'],
            env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
        until(lambda: int(ask('get ticks')) >= 8, 'Luau/IPC did not remain live without an output')
        initial = phase('initially absent')
        assert initial['native_hwnds'] == 0 and initial['screens.count'] == '0'
        connected(True)
        until(lambda: smoke.native_window_count(process.pid) == 4 and 'first frame' in (out / 'native.log').read_text(encoding='utf-8'), 'Native panels did not present')
        present = phase('two panels on one output')
        assert present['screens.count'] == '1', f"Two panels on one monitor reported screens.count={present['screens.count']}"
        ask('fact open true')
        until(lambda: smoke.native_window_count(process.pid) == 6, 'Popup did not open')
        phase('popup open')
        for cycle in range(3):
            connected(False)
            until(lambda: smoke.native_window_count(process.pid, visible_only=False) == 0, 'Removed HWNDs were not destroyed after renderer release')
            absent = phase(f'absent {cycle}')
            assert absent['screens.count'] == '0', 'Removed output retained a nonzero screens.count'
            previous_ticks = int(absent['ticks'])
            until(lambda: int(ask('get ticks')) >= previous_ticks + 4, 'Luau stopped after output removal')
            connected(True)
            until(lambda: smoke.native_window_count(process.pid) == 6, 'Panels/popup did not recover')
            restored = phase(f'restored {cycle}')
            assert restored['screens.count'] == '1'
        ask('fact open false')
        until(lambda: smoke.native_window_count(process.pid, visible_only=False) == 4, 'Popup resources were not released')
        phase('ready for input')
        if args.interactive:
            print('READY: click the main Monitor lifecycle panel after reconnection.', flush=True)
            until(lambda: int(ask('get clicks')) >= 1, 'Physical click after reconnection not received', seconds=120)
            phase('physical click after reconnection')
        ask('quit')
        process.wait(timeout=15)
        assert process.returncode == 0
        assert smoke.native_window_count(process.pid, visible_only=False) == 0
        report['complete'] = True
except BaseException as error:
    report['failure'] = str(error)
    raise
finally:
    if process is not None and process.poll() is None:
        try: ask('quit'); process.wait(timeout=15)
        except subprocess.SubprocessError: process.kill(); process.wait(); report['forced_cleanup'] = True
    report['exit_code'] = process.returncode if process is not None else None
    (out / 'report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
