"""Check real native surface/popup capture affinity and reload on non-primary DISPLAY2.

No mouse/keyboard input or desktop screenshot is taken. OS affinity readback
does not verify every third-party recorder or remote-desktop implementation.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
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
    assert os.name == 'nt', 'Native Windows is required'
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    screen = r'\\.\DISPLAY2'
    user = c.WinDLL('user32', use_last_error=True)
    user.SetProcessDpiAwarenessContext.argtypes = [w.HANDLE]
    user.SetProcessDpiAwarenessContext(w.HANDLE(-4))

    class Monitor(c.Structure):
        _fields_ = [('size', w.DWORD), ('rect', w.RECT), ('work', w.RECT),
                    ('flags', w.DWORD), ('name', w.WCHAR * 32)]

    monitor_callback = c.WINFUNCTYPE(w.BOOL, w.HANDLE, w.HDC, c.POINTER(w.RECT), w.LPARAM)
    user.GetMonitorInfoW.argtypes = [w.HANDLE, c.POINTER(Monitor)]
    user.EnumDisplayMonitors.argtypes = [w.HDC, c.POINTER(w.RECT), monitor_callback, w.LPARAM]
    monitors = []
    def visit_monitor(handle, _dc, _rect, _data):
        info = Monitor(); info.size = c.sizeof(info)
        if user.GetMonitorInfoW(handle, c.byref(info)): monitors.append(info)
        return True
    assert user.EnumDisplayMonitors(None, None, monitor_callback(visit_monitor), 0)
    monitor = next((m for m in monitors if m.name == screen and not m.flags & 1), None)
    assert monitor is not None, 'Active non-primary DISPLAY2 is required'
    assert monitor.rect.right - monitor.rect.left >= 800 and monitor.rect.bottom - monitor.rect.top >= 600

    enum_callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    user.EnumWindows.argtypes = [enum_callback, w.LPARAM]
    user.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
    user.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
    user.GetWindowDisplayAffinity.argtypes = [w.HWND, c.POINTER(w.DWORD)]
    user.IsWindowVisible.argtypes = [w.HWND]
    user.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
    user.GetForegroundWindow.restype = w.HWND
    foreground = user.GetForegroundWindow()
    output.mkdir(parents=True, exist_ok=False)
    scene = output / 'capture-visibility.plm'
    env = dict(os.environ, APPDATA=str(output / 'state'), PLEAMAR_SOCKET_DIR=f'capture-policy-{os.getpid()}',
               PLEAMAR_NO_RELAUNCH='1', PLEAMAR_TEST_WINDOWS='1')
    flags = subprocess.CREATE_NO_WINDOW | subprocess.BELOW_NORMAL_PRIORITY_CLASS
    report = {'complete': False, 'physical_input': False, 'screen': screen,
              'pixel_capture_validation': False, 'stages': [],
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    process = None

    def publish(hidden, revision):
        source = f'''scene CaptureVisibility {{
            surface {{ size: 400, 250; anchor: top_left; margin: 80; keyboard: none;
                      captures: {'hidden' if hidden else 'shown'} }}
            fact revision_{revision} = true
            fact expanded = false
            box {{ from: 0, 0; size: 400, 250; color: #182c36 }}
            text "Native capture visibility" {{ at: 20, 28; size: 16; color: #ffffff }}
            popup child {{ at: 30, 65; size: 160, 90; open: expanded
                box {{ from: 0, 0; size: 160, 90; color: #507060 }}
            }}
        }}'''
        temporary = scene.with_suffix('.new')
        temporary.write_text(source, encoding='utf-8')
        temporary.replace(scene)

    def ask(command):
        return subprocess.check_output([str(binary), '--say', scene.stem, command], env=env,
            encoding='utf-8', stderr=subprocess.DEVNULL, timeout=5, creationflags=flags).strip()

    def windows():
        found, errors = [], []
        def visit(hwnd, _):
            owner = w.DWORD()
            user.GetWindowThreadProcessId(hwnd, c.byref(owner))
            if owner.value != process.pid or not user.IsWindowVisible(hwnd): return True
            name = c.create_unicode_buffer(128)
            user.GetClassNameW(hwnd, name, len(name))
            if name.value != 'PleamarWindow': return True
            rect, affinity = w.RECT(), w.DWORD()
            if not user.GetWindowRect(hwnd, c.byref(rect)) or not user.GetWindowDisplayAffinity(hwnd, c.byref(affinity)):
                errors.append(c.get_last_error()); return True
            inside = (rect.left >= monitor.rect.left and rect.top >= monitor.rect.top
                      and rect.right <= monitor.rect.right and rect.bottom <= monitor.rect.bottom)
            if not inside: errors.append('Owned window left DISPLAY2')
            found.append({'affinity': affinity.value, 'rect': [rect.left, rect.top, rect.right, rect.bottom]})
            return True
        assert user.EnumWindows(enum_callback(visit), 0)
        if errors: raise RuntimeError(str(errors))
        return found

    def until(predicate, label):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if process.poll() is not None: raise RuntimeError(f'Process exited during {label}')
            try:
                if predicate(): return
            except (subprocess.SubprocessError, ValueError): pass
            time.sleep(.1)
        raise TimeoutError(label)

    def verify(label, count, value):
        def matches():
            actual = windows()
            return len(actual) == count and all(v['affinity'] == value for v in actual)
        until(matches, label)
        report['stages'].append({'name': label, 'windows': windows()})

    try:
        publish(True, 1)
        with (output / 'native.log').open('w', encoding='utf-8') as log:
            process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen', screen,
                '--no-hud', '--stall', '0', '--seconds', '120'], env=env, stdout=log, stderr=log, creationflags=flags)
            until(lambda: ask('get revision_1') == 'true', 'startup')
            verify('hidden native canvas and input proxy', 2, 0x11)
            ask('fact expanded true')
            verify('new popup inherits hidden capture policy', 4, 0x11)
            publish(False, 2)
            until(lambda: ask('get revision_2') == 'true', 'shown reload')
            verify('reload restores capture of existing surface and popup', 4, 0)
            publish(True, 3)
            until(lambda: ask('get revision_3') == 'true', 'hidden reload')
            verify('reload excludes existing surface and popup', 4, 0x11)
            ask('fact expanded false')
            verify('closed popup released', 2, 0x11)
            ask('fact expanded true')
            verify('recreated popup inherits current policy', 4, 0x11)
            ask('quit'); process.wait(timeout=15)
            assert process.returncode == 0
            report['foreground_unchanged'] = user.GetForegroundWindow() == foreground
            report['complete'] = True
    except BaseException as error:
        report['failure'] = str(error)
        raise
    finally:
        if process and process.poll() is None:
            try: ask('quit'); process.wait(timeout=15)
            except subprocess.SubprocessError: process.kill(); process.wait(timeout=10)
        report['exit_code'] = process.returncode if process else None
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print('PASS: native capture affinity, input proxy, popup inheritance and hot reload on DISPLAY2; no physical input or capture pixels.')


if __name__ == '__main__': main()
