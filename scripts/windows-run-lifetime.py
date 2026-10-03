"""Verify native run() children stop on Luau reload, without creating a window."""
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
    args = parser.parse_args()
    if os.name != 'nt': parser.error('This fixture uses native PowerShell helpers.')
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    scene = output / 'run-lifetime.plm'
    logic = scene.with_suffix('.luau')
    environment = dict(os.environ, APPDATA=str(output / 'state'),
                       PLEAMAR_SOCKET_DIR=f'run-lifetime-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1')
    report = dict(passed=False, graphical_validation=False, stages=[],
                  binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.WaitForSingleObject.restype = wintypes.DWORD
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    process, handle = None, None

    def ask(command):
        result = subprocess.run([str(binary), '--say', scene.stem, command], env=environment,
            capture_output=True, encoding='utf-8', timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode: raise RuntimeError(result.stdout + result.stderr)
        return result.stdout.strip()

    def until(predicate, label):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if process.poll() is not None: raise RuntimeError(f'Process exited during {label}')
            try:
                if predicate(): return
            except RuntimeError: pass
            time.sleep(.05)
        raise TimeoutError(label)

    def write_logic(source):
        temporary = logic.with_suffix('.tmp')
        temporary.write_text(source, encoding='utf-8')
        temporary.replace(logic)

    try:
        scene.write_text('''scene RunLifetime {
            surface { kind: window; size: 100, 100 }
            permissions { run: "powershell.exe" }
            fact revision = 0
            fact replies = 0
        }''', encoding='utf-8')
        write_logic('fact.revision = 1\n')
        with (output / 'native.log').open('w', encoding='utf-8') as log:
            process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen',
                f'pleamar-absent-run-test-{os.getpid()}', '--no-hud', '--stall', '0', '--seconds', '60'],
                env=environment, stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
            until(lambda: ask('get revision') == '1', 'initial Lua state')
            for index, collect in enumerate([True, False]):
                folder = output / f'helper {index} ñ'
                folder.mkdir()
                (folder / 'helper.ps1').write_text('''$ErrorActionPreference = 'Stop'
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'ready'), [string]$PID)
Start-Sleep -Seconds 12
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'finished'), 'outlived its logic')
''', encoding='utf-8-sig')
                revision = 2 + index * 2
                callback = 'function() fact.replies += 1 end' if collect else 'nil'
                write_logic(f'''run("powershell.exe", {{"-NoProfile", "-NonInteractive", "-File", "helper.ps1"}},
                    {callback}, {{ cwd = [==[{folder}]==], output = {str(collect).lower()} }})
                    fact.revision = {revision}
                ''')
                until(lambda: (folder / 'ready').is_file(), 'owned helper started')
                child_pid = int((folder / 'ready').read_text())
                handle = kernel.OpenProcess(0x100000, False, child_pid)  # SYNCHRONIZE only
                if not handle: raise ctypes.WinError(ctypes.get_last_error())
                assert kernel.WaitForSingleObject(handle, 0) == 258, 'helper exited before reload'
                started = time.monotonic()
                write_logic(f'fact.revision = {revision + 1}\n')
                until(lambda: ask('get revision') == str(revision + 1), 'replacement Lua state')
                wait = kernel.WaitForSingleObject(handle, 3000)
                report['stages'].append(dict(collect_output=collect, helper_pid=child_pid,
                    wait_result=wait, reload_and_exit_ms=round((time.monotonic() - started) * 1000, 1)))
                assert wait == 0, 'run helper remained alive after its logic reloaded'
                kernel.CloseHandle(handle)
                handle = None
                assert not (folder / 'finished').exists(), 'retired helper kept writing'
                assert ask('get replies') == '0', 'retired callback ran in the replacement VM'
            ask('quit')
            report['exit_code'] = process.wait(timeout=10)
            assert report['exit_code'] == 0
        trace = (output / 'native.log').read_text(encoding='utf-8')
        assert 'waiting for one to appear' in trace and 'first frame' not in trace and 'panicked' not in trace
        report['passed'] = True
    finally:
        # Quit only this fixture; Windows' owned Job Object cleans its children.
        if process and process.poll() is None:
            try: ask('quit')
            except Exception: process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
        if handle: kernel.CloseHandle(handle)
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print('PASS: native run helpers stop on reload with and without output/callbacks; no display selected')


if __name__ == '__main__': main()
