"""Observe the native audio watcher and Lua reloads without changing devices or volume."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--seconds', type=int, default=30)
    args = parser.parse_args()
    if os.name != 'nt': parser.error('Native Windows only')
    if not 5 <= args.seconds <= 300: parser.error('seconds must be 5..300')
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    scene = output / 'audio-watch.plm'
    logic = scene.with_suffix('.luau')
    environment = dict(os.environ, APPDATA=str(output / 'state'),
                       PLEAMAR_SOCKET_DIR=f'audio-watch-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1')
    report = dict(passed=False, graphical_validation=False, changes_to_audio_settings=False, reloads=[],
                  binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), logical_cpus=os.cpu_count())
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    kernel.GetProcessTimes.restype = wintypes.BOOL
    kernel.GetProcessHandleCount.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel.GetProcessHandleCount.restype = wintypes.BOOL
    process = None

    def metrics():
        created, exited, system, user = [wintypes.FILETIME() for _ in range(4)]
        handles = wintypes.DWORD()
        for success in [kernel.GetProcessTimes(int(process._handle), ctypes.byref(created), ctypes.byref(exited), ctypes.byref(system), ctypes.byref(user)),
                        kernel.GetProcessHandleCount(int(process._handle), ctypes.byref(handles))]:
            if not success: raise ctypes.WinError(ctypes.get_last_error())
        return dict(cpu_seconds=sum((v.dwHighDateTime << 32) | v.dwLowDateTime for v in [system, user]) / 1e7,
                    handles=handles.value)

    def ask(command):
        result = subprocess.run([str(binary), '--say', scene.stem, command], env=environment,
            capture_output=True, encoding='utf-8', timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode: raise RuntimeError(result.stdout + result.stderr)
        return result.stdout.strip()

    def until(predicate, label):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if process.poll() is not None: raise RuntimeError(f'Process exited during {label}')
            try:
                if predicate(): return
            except RuntimeError: pass
            time.sleep(.05)
        raise TimeoutError(label)

    def write_logic(generation):
        temporary = logic.with_suffix('.tmp')
        temporary.write_text('''assert(sys.watch("audio", function(value)
            fact.outputs = #(value.outputs or {})
            fact.inputs = #(value.inputs or {})
            fact.volume = tonumber(value.volume) or -1
            fact.input = tonumber(value.input) or -1
            fact.available = value.available ~= false
            fact.snapshots += 1
            fact.generation = GENERATION
        end))
'''.replace('GENERATION', str(generation)), encoding='utf-8')
        temporary.replace(logic)

    def state():
        return {name: ask('get ' + name) for name in ['generation', 'snapshots', 'outputs', 'inputs', 'volume', 'input', 'available']}

    try:
        scene.write_text('''scene AudioWatch {
            surface { kind: window; size: 100, 100 }
            permissions { services: "audio" }
            fact generation = 0
            fact snapshots = 0
            fact outputs = 0
            fact inputs = 0
            fact volume = -1
            fact input = -1
            fact available = false
        }''', encoding='utf-8')
        write_logic(1)
        with (output / 'native.log').open('w', encoding='utf-8') as log:
            process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen',
                f'pleamar-absent-audio-test-{os.getpid()}', '--no-hud', '--stall', '0', '--seconds', str(args.seconds + 90)],
                env=environment, stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
            until(lambda: ask('get generation') == '1', 'initial audio snapshot')
            time.sleep(5)
            report['initial_state'] = state()
            before = metrics()
            started = time.monotonic()
            time.sleep(args.seconds)
            elapsed = time.monotonic() - started
            after = metrics()
            report['observation'] = dict(before=before, after=after, seconds=elapsed,
                cpu_percent_one_core=100 * (after['cpu_seconds'] - before['cpu_seconds']) / elapsed)
            report['observed_state'] = state()
            for generation in range(2, 7):
                write_logic(generation)
                until(lambda: ask('get generation') == str(generation), f'reload {generation}')
                report['reloads'].append(state())
            ask('quit')
            report['exit_code'] = process.wait(timeout=10)
            assert report['exit_code'] == 0
        trace = (output / 'native.log').read_text(encoding='utf-8')
        assert 'waiting for one to appear' in trace and 'first frame' not in trace
        assert 'panicked' not in trace and 'runtime error:' not in trace
        report['notification_fallback'] = 'audio notifications:' in trace
        report['passed'] = True
    finally:
        if process and process.poll() is None:
            try: ask('quit')
            except Exception: process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait(timeout=10)
        report['scope'] = 'Read-only audio snapshots and five Lua reloads; absent output. Process CPU includes runtime overhead. No device-change, volume-change latency, sound quality or graphical validation.'
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(dict(passed=report['passed'], observation=report['observation'],
                         notification_fallback=report['notification_fallback'], initial_state=report['initial_state'])))


if __name__ == '__main__': main()
