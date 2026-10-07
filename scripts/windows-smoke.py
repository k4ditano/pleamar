#!/usr/bin/env python3
"""CLI checks; --gui additionally starts native windows (requires a desktop/GPU)."""
import argparse
import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def monitor_snapshot():
    """Read monitor work areas; this does not inject input or change settings."""
    class Info(ctypes.Structure):
        _fields_ = [('size', wintypes.DWORD), ('monitor', wintypes.RECT), ('work', wintypes.RECT), ('flags', wintypes.DWORD), ('name', wintypes.WCHAR * 32)]
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HANDLE, wintypes.HDC, ctypes.POINTER(wintypes.RECT), wintypes.LPARAM)
    user32.GetMonitorInfoW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Info)]
    result = {}
    def visit(handle, _dc, _rect, _data):
        info = Info()
        info.size = ctypes.sizeof(info)
        assert user32.GetMonitorInfoW(handle, ctypes.byref(info))
        result[info.name] = (info.work.left, info.work.top, info.work.right, info.work.bottom)
        return True
    assert user32.EnumDisplayMonitors(None, None, callback_type(visit), 0)
    return result


def native_window_count(pid, visible_only=True):
    """Count pleamar HWNDs owned by this test process, optionally including hidden ones."""
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    count = 0
    def visit(hwnd, _data):
        nonlocal count
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and (not visible_only or user32.IsWindowVisible(hwnd)):
            name = ctypes.create_unicode_buffer(128)
            if user32.GetClassNameW(hwnd, name, len(name)) and name.value == 'PleamarWindow':
                count += 1
        return True
    assert user32.EnumWindows(callback_type(visit), 0)
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--gui', action='store_true')
    args = parser.parse_args()
    binary = args.binary.resolve()
    assert os.name == 'nt', 'Windows native smoke test'
    environment = dict(os.environ, PLEAMAR_SOCKET_DIR=f'smoke-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1', PLEAMAR_VALIDATE='1')

    def run(*cmd):
        result = subprocess.run([str(binary), *map(str, cmd)], env=environment, capture_output=True, encoding='utf-8', timeout=10)
        assert result.returncode == 0, result.stdout + result.stderr
        return result.stdout.strip()

    assert 'pleamar ' in run('--version')
    assert ': ok ·' in run('--check', ROOT / 'tests/native-interaction.plm')
    with tempfile.TemporaryDirectory(prefix='pleamar autostart ñ ') as directory:
        config = Path(directory)
        result = config / 'salida Unicode.txt'
        literal = str(result).replace("'", "''")
        command = f"[IO.File]::WriteAllText('{literal}', 'España ñ 世界', [Text.Encoding]::UTF8)"
        (config / 'autostart').write_text('# ignored\nwm: ignored\n' + command + '\n', encoding='utf-8')
        environment['PLEAMAR_CONFIG'] = directory
        run('--autostart')
        deadline = time.monotonic() + 10
        while not result.exists() and time.monotonic() < deadline: time.sleep(.1)
        assert result.read_text(encoding='utf-8-sig') == 'España ñ 世界'
        del environment['PLEAMAR_CONFIG']
    print('PASS: native CLI, scene syntax, PowerShell autostart with Unicode config/output paths')

    # No surface/GPU presentation is needed to keep Luau and IPC usable while
    # an explicitly selected output is unplugged. Exercise both exit paths.
    with tempfile.TemporaryDirectory(prefix='pleamar absent output ñ ') as directory:
        for timed in (False, True):
            log = Path(directory) / f'waiting-{timed}.log'
            with log.open('w', encoding='utf-8') as output:
                process = subprocess.Popen([str(binary), '--scene', str(ROOT / 'tests/native-display-lifecycle.plm'),
                    '--screen', f'pleamar-absent-test-{os.getpid()}', '--no-hud', '--stall', '0', '--seconds', '3' if timed else '20'],
                    env=environment, stdout=output, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
                try:
                    deadline = time.monotonic() + 2.5
                    ready = False
                    while time.monotonic() < deadline and process.poll() is None:
                        try:
                            ready = int(run('--say', 'native-display-lifecycle', 'get ticks')) >= 2
                            if ready: break
                        except (AssertionError, ValueError): pass
                        time.sleep(.1)
                    assert ready, log.read_text(encoding='utf-8')
                    assert native_window_count(process.pid, visible_only=False) == 0
                    if not timed:
                        report = Path(directory) / 'informe ñ.md'
                        environment['PLEAMAR_DIAGNOSTIC_PATH'] = directory
                        run('--report', '--seconds', '5', '--out', report)
                        del environment['PLEAMAR_DIAGNOSTIC_PATH']
                        measured = report.read_text(encoding='utf-8')
                        assert 'native-display-lifecycle' in measured and 'Windows desktop' in measured
                        assert 'CPU busy (whole system)' in measured and 'Memory (RSS)' in measured
                        assert 'PLEAMAR_DIAGNOSTIC_PATH (a path)' in measured and directory not in measured
                        assert 'Unavailable counters' in measured
                        assert native_window_count(process.pid, visible_only=False) == 0
                        run('--say', 'native-display-lifecycle', 'quit')
                    assert process.wait(timeout=8) == 0
                    text = log.read_text(encoding='utf-8')
                    assert 'waiting for one to appear' in text and 'first frame' not in text, text
                finally:
                    if process.poll() is None: process.kill(); process.wait(timeout=10)
    print('PASS: absent output keeps native Luau/IPC live without HWNDs; IPC and timed exit both close cleanly')
    print('PASS: report discovers native pipes, samples real CPU/memory, writes Unicode paths and redacts Windows paths; no rendered-frame validation')
    if not args.gui:
        print('NOT RUN: graphical smoke (use --gui on an interactive Windows desktop)')
        return

    # Match the runtime's physical-coordinate monitor observations.
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    primary_test_monitor = next(iter(monitor_snapshot()))

    with tempfile.TemporaryDirectory(prefix='pleamar prueba ñ ') as directory:
        scene = Path(directory) / 'native-interaction.plm'
        logic = scene.with_suffix('.luau')
        original = (ROOT / 'tests/native-interaction.plm').read_text(encoding='utf-8')
        scene.write_text(original, encoding='utf-8')
        original_logic = (ROOT / 'tests/native-interaction.luau').read_text(encoding='utf-8')
        logic.write_text(original_logic, encoding='utf-8')
        log = Path(directory) / 'runtime.log'
        with log.open('w', encoding='utf-8') as output:
            # A repeated output asks for extra panel copies, but a decorated
            # scene still owns one window. This catches duplication on one GPU.
            process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen', f'{primary_test_monitor},{primary_test_monitor}', '--no-hud', '--stall', '0', '--seconds', '35'], env=environment, stdout=output, stderr=subprocess.STDOUT)
            def ask(command):
                return run('--say', 'native-interaction', command)
            def until(check, label, seconds=10):
                end = time.monotonic() + seconds
                while time.monotonic() < end:
                    try:
                        if check(): return
                    except (AssertionError, ValueError): pass
                    if process.poll() is not None: raise AssertionError(log.read_text(encoding='utf-8'))
                    time.sleep(.2)
                raise AssertionError(label + '\n' + log.read_text(encoding='utf-8'))
            try:
                until(lambda: 'first frame' in log.read_text(encoding='utf-8'), 'native GPU did not present')
                until(lambda: int(ask('get ticks')) >= 8, 'Luau timer and monitor reconciliation')
                count = native_window_count(process.pid)
                assert count == 1, f'One window surface created {count} native HWNDs for repeated outputs'
                print('PASS: one actual native window for repeated output selectors')
                ask('emit clicked')
                until(lambda: ask('get clicks') == '1', 'Luau event')
                ask('submit entry España ñ 世界 🚀')
                until(lambda: ask('get result') == 'España ñ 世界 🚀', 'Unicode submit')
                scene.write_text(original.replace('Native Windows', 'Reloaded Windows'), encoding='utf-8')
                until(lambda: 'reload ·' in log.read_text(encoding='utf-8'), 'scene reload')
                scene.write_text(original + '\ninvalid syntax', encoding='utf-8')
                until(lambda: 'scene stays as it was' in log.read_text(encoding='utf-8'), 'invalid scene fallback')
                assert ask('get clicks') == '1'
                scene.write_text(original, encoding='utf-8')
                logic.write_text(original_logic + '\ntext.result = "Luau reloaded"\n', encoding='utf-8')
                until(lambda: ask('get result') == 'Luau reloaded', 'Luau reload')
                ask('fact open true')
                until(lambda: 'surface 1 on' in log.read_text(encoding='utf-8') and ask('get open') == 'true', 'popup sheet opened')
                ask('fact open false')
                ask('quit')
                assert process.wait(timeout=10) == 0
                assert 'panicked' not in log.read_text(encoding='utf-8')
                print('PASS: real HWND/DX12 presentation, Luau timer/event, IPC, Unicode submit, scene/error/logic reload, popup request, graceful quit')
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
        print(log.read_text(encoding='utf-8'))
        before = monitor_snapshot()
        name = next(iter(before))
        bar = Path(directory) / 'appbar-check.plm'
        bar.write_text('scene AppbarCheck {\n surface { size: full, 72; anchor: top; reserve: 72 while keeps }\n fact keeps = false\n box { from: 0, 0; size: screen.width, 72; color: #123456 }\n}', encoding='utf-8')
        run('--check', bar)
        with log.open('w', encoding='utf-8') as output:
            process = subprocess.Popen([str(binary), '--scene', str(bar), '--screen', name, '--no-hud', '--seconds', '10'], env=environment, stdout=output, stderr=subprocess.STDOUT)
            try:
                until(lambda: 'first frame' in log.read_text(encoding='utf-8'), 'AppBar first frame')
                assert monitor_snapshot() == before, 'conditional reservation must start disabled'
                run('--say', 'appbar-check', 'fact keeps true')
                until(lambda: monitor_snapshot()[name][1] > before[name][1], 'AppBar did not reserve desktop space')
                during = monitor_snapshot()[name]
                run('--say', 'appbar-check', 'fact keeps false')
                until(lambda: monitor_snapshot() == before, 'disabled reservation did not restore the work area')
                run('--say', 'appbar-check', 'fact keeps true')
                until(lambda: monitor_snapshot()[name] == during, 'reservation did not resume')
                assert process.wait(timeout=12) == 0
                assert monitor_snapshot() == before, 'AppBar work area was not restored on timed exit'
                print(f'PASS: AppBar conditional reserve/toggle/timed exit {name}: before={before[name]}, during={during}, restored={monitor_snapshot()[name]}')
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
    print('NOTE: IPC drives this smoke; physical input and visual correctness require the manual checklist in docs/windows.md')


if __name__ == '__main__':
    main()
