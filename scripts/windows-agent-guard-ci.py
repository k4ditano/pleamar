"""Named input guards on an owned window on a disposable CI desktop.

No OS mouse/keyboard input or real applications are used. Never run locally.
"""
from pathlib import Path
import argparse
import ctypes as C
from ctypes import wintypes as W
import hashlib
import json
import os
import struct
import subprocess
import sys
import time
import zlib


def require_ci():
    if (sys.platform != 'win32' or os.environ.get('GITHUB_ACTIONS') != 'true'
            or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted'
            or os.environ.get('PLEAMAR_CI_AGENT_GUARD') != '1'):
        raise RuntimeError('Visible input-guard fixture requires the explicit step on a disposable GitHub-hosted Windows runner')


class Desktop:
    def __init__(self):
        self.user = C.WinDLL('user32', use_last_error=True)
        self.gdi = C.WinDLL('gdi32', use_last_error=True)
        self.callback = C.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
        for lib, name, result, args in [
            (self.user, 'EnumWindows', W.BOOL, [self.callback, W.LPARAM]),
            (self.user, 'GetWindowThreadProcessId', W.DWORD, [W.HWND, C.POINTER(W.DWORD)]),
            (self.user, 'GetWindowTextW', C.c_int, [W.HWND, W.LPWSTR, C.c_int]),
            (self.user, 'IsWindowVisible', W.BOOL, [W.HWND]),
            (self.user, 'SetProcessDpiAwarenessContext', W.BOOL, [W.HANDLE]),
            (self.user, 'GetClientRect', W.BOOL, [W.HWND, C.POINTER(W.RECT)]),
            (self.user, 'ClientToScreen', W.BOOL, [W.HWND, C.POINTER(W.POINT)]),
            (self.user, 'GetDC', W.HDC, [W.HWND]),
            (self.user, 'ReleaseDC', C.c_int, [W.HWND, W.HDC]),
            (self.user, 'PostMessageW', W.BOOL, [W.HWND, W.UINT, W.WPARAM, W.LPARAM]),
            (self.gdi, 'CreateCompatibleDC', W.HDC, [W.HDC]),
            (self.gdi, 'CreateCompatibleBitmap', W.HBITMAP, [W.HDC, C.c_int, C.c_int]),
            (self.gdi, 'SelectObject', W.HANDLE, [W.HDC, W.HANDLE]),
            (self.gdi, 'BitBlt', W.BOOL, [W.HDC, C.c_int, C.c_int, C.c_int, C.c_int, W.HDC, C.c_int, C.c_int, W.DWORD]),
            (self.gdi, 'GetDIBits', C.c_int, [W.HDC, W.HBITMAP, W.UINT, W.UINT, C.c_void_p, C.c_void_p, W.UINT]),
            (self.gdi, 'DeleteObject', W.BOOL, [W.HANDLE]),
            (self.gdi, 'DeleteDC', W.BOOL, [W.HDC]),
        ]:
            function = getattr(lib, name)
            function.restype, function.argtypes = result, args
        if not self.user.SetProcessDpiAwarenessContext(W.HANDLE(-4)):
            raise C.WinError(C.get_last_error())

    def pid(self, hwnd):
        pid = W.DWORD()
        assert self.user.GetWindowThreadProcessId(hwnd, C.byref(pid))
        return pid.value

    def window(self, pid, title):
        found = []

        @self.callback
        def visit(hwnd, _):
            text = C.create_unicode_buffer(256)
            self.user.GetWindowTextW(hwnd, text, len(text))
            if text.value.startswith(title) and self.pid(hwnd) == pid and self.user.IsWindowVisible(hwnd):
                found.append(hwnd)
            return True

        assert self.user.EnumWindows(visit, 0)
        assert len(found) <= 1
        return found[0] if found else None

    def pixels(self, hwnd):
        rect, point = W.RECT(), W.POINT()
        assert self.user.GetClientRect(hwnd, C.byref(rect))
        assert self.user.ClientToScreen(hwnd, C.byref(point))
        width, height = rect.right, rect.bottom
        assert 100 <= width <= 4096 and 100 <= height <= 2160
        source = self.user.GetDC(None)
        target = self.gdi.CreateCompatibleDC(source)
        bitmap = self.gdi.CreateCompatibleBitmap(source, width, height)
        assert source and target and bitmap
        old = self.gdi.SelectObject(target, bitmap)
        try:
            # Include the native transparent panel in the runner's composed image.
            assert self.gdi.BitBlt(target, 0, 0, width, height, source, point.x, point.y, 0x40CC0020)
            self.gdi.SelectObject(target, old)
            old = None
            header = C.create_string_buffer(struct.pack('<IiiHHIIiiII', 40, width, -height, 1, 32, 0, 0, 0, 0, 0, 0))
            pixels = C.create_string_buffer(width * height * 4)
            assert self.gdi.GetDIBits(target, bitmap, 0, height, pixels, header, 0) == height
            return width, height, pixels.raw
        finally:
            if old:
                self.gdi.SelectObject(target, old)
            self.gdi.DeleteObject(bitmap)
            self.gdi.DeleteDC(target)
            self.user.ReleaseDC(None, source)


def png(path, picture):
    width, height, bgra = picture
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        row = bgra[y * width * 4:(y + 1) * width * 4]
        for i in range(0, len(row), 4):
            rows.extend((row[i + 2], row[i + 1], row[i]))

    def chunk(name, data):
        return struct.pack('>I', len(data)) + name + data + struct.pack('>I', zlib.crc32(name + data))

    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def exercise(binary, output):
    root = Path(__file__).resolve().parents[1]
    scene = output / 'Changing target ñ 海.plm'
    scene.write_bytes((root / 'tests/agent-changing-target.plm').read_bytes())
    desktop = Desktop()
    desktop.user.GetForegroundWindow.restype = W.HWND
    desktop.user.GetForegroundWindow.argtypes = []
    foreground = desktop.user.GetForegroundWindow()
    env = dict(os.environ, APPDATA=str(output / 'state'),
               PLEAMAR_CONFIG=str(output / 'config'), PLEAMAR_SOCKET_DIR=f'agent-guard-ci-{os.getpid()}',
               PLEAMAR_NO_RELAUNCH='1', PLEAMAR_TEST_WINDOWS='1')
    flags = subprocess.CREATE_NO_WINDOW | subprocess.BELOW_NORMAL_PRIORITY_CLASS
    report = dict(passed=False, environment='github-hosted', physical_input=False,
                  full_product_acceptance=False, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  checks=[], images=[])
    process, hwnd = None, None

    def run(arguments):
        result = subprocess.run([str(binary), *arguments], env=env, capture_output=True,
                                encoding='utf-8', errors='replace', timeout=15, creationflags=flags)
        with (output / 'commands.log').open('a', encoding='utf-8') as log:
            log.write(json.dumps(arguments, ensure_ascii=False) + '\n' + result.stdout + result.stderr)
        assert result.returncode == 0, result.stdout + result.stderr
        return result.stdout.strip()

    def ask(command):
        return run(['--say', scene.stem, command])

    def until(predicate):
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            assert process.poll() is None, 'Owned scene stopped'
            try:
                if predicate(): return
            except (OSError, AssertionError): pass
            time.sleep(.1)
        raise TimeoutError('Owned input-guard scene did not become ready')

    def capture(label, guard=False):
        picture = desktop.pixels(hwnd)
        width, height, bgra = picture
        assert width >= 420 and height >= 300
        # The blocker is a flat red rectangle; verify real composed pixels.
        if guard:
            count = sum(all(abs(a-b) <= 5 for a,b in zip(bgra[i:i+3], (0x50,0x50,0x80)))
                        for i in range(0,len(bgra),4))
            assert count > 5000, 'The person-only blocker was not rendered'
        path = output / (label + '.png'); png(path,picture)
        report['images'].append(dict(file=path.name, sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
        (output / (label + '.json')).write_text(ask('describe json'),encoding='utf-8')

    try:
        assert ': ok' in run(['--check', str(scene)])
        with (output / 'scene.log').open('w',encoding='utf-8') as log:
            process = subprocess.Popen([str(binary),'--scene',str(scene),'--no-hud','--stall','0','--seconds','90'],
                                       env=env, stdout=log,stderr=log,creationflags=flags)
            until(lambda: ask('get blocked') == 'false')
            until(lambda: desktop.window(process.pid,'pleamar surface 0 · ') is not None)
            hwnd = desktop.window(process.pid,'pleamar surface 0 · ')
            assert ask('type query Original ñ').startswith('typed'), 'Uncovered input rejected text'
            assert ask('get query') == 'Original ñ'
            capture('01-uncovered')
            answer = ask('press target')
            assert answer.startswith('?') and 'does not answer' not in answer, answer
            assert ask('get blocked') == 'true', 'Hover did not expose the changing target'
            assert ask('get allowed') == 'false' and ask('get forbidden') == 'false'
            report['checks'].append('Hover blocker rejects the stale press without activating either target')
            answer = ask('key !')
            assert answer.startswith('?') and 'does not answer' not in answer, answer
            assert ask('get query') == 'Original ñ', 'Covered input accepted a key'
            time.sleep(.2); capture('02-blocked',guard=True)
            report['checks'].append('A previously focused field rejects keys after it is covered')
            assert not ask('fact blocked false').startswith('?')
            until(lambda: ask('get blocked') == 'false')
            assert ask('type query Recovered ñ').startswith('typed')
            assert ask('get query') == 'Recovered ñ'
            capture('03-recovered')
            report['checks'].append('Removing the blocker restores named input')
            assert not ask('quit').startswith('?')
            assert process.wait(timeout=15) == 0
        logs = (output / 'scene.log').read_text(encoding='utf-8')
        assert 'first frame' in logs and 'panicked' not in logs
        report['foreground_unchanged'] = foreground == desktop.user.GetForegroundWindow()
        assert report['foreground_unchanged']
        report['passed'] = True
    finally:
        if process and process.poll() is None:
            if hwnd: desktop.user.PostMessageW(hwnd,0x10,0,0)
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=10)
        (output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report,indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    require_ci()
    temporary = Path(os.environ['RUNNER_TEMP']).resolve(); output = args.output.resolve()
    assert output != temporary and output.is_relative_to(temporary)
    output.mkdir(parents=True,exist_ok=False)
    exercise(args.binary.resolve(strict=True),output)


if __name__ == '__main__': main()
