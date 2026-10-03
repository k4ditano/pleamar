#!/usr/bin/env python3
"""Exercise native hit regions and hot reload on one explicitly selected output.

Input is injected into the renderer, not the desktop. The owned panel never asks
for keyboard focus, reserves no work area and uses no system services.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--screen', required=True, help=r'Exact monitor name, for example \\.\\DISPLAY2')
    args = parser.parse_args()
    assert os.name == 'nt', 'Requires native Windows rendering'
    assert args.screen and not any(c in args.screen for c in '\r\n"'), 'A literal monitor name is required'
    binary = args.binary.resolve(strict=True)
    work = Path(tempfile.mkdtemp(prefix='pleamar geometry '))
    scene = work / 'geometry.plm'
    source = '''scene Geometry {
    surface { size: 400, 280; screens: MONITOR; keyboard: none; level: overlay; rate: 60 }
    fact clicks = 0
    fact offset = 0
    fact enabled = true
    box { from: 0, 0; size: 400, 280; color: #142224 }
    group {
        z: offset
        opacity: if(enabled, 1, 0)
        box button { from: 40 + offset, 50; size: 80, 40; color: #80dcc2; active: enabled }
    }
    text "Geometry cache validation" { at: 20, 15; size: 18; color: #ffffff }
    on press button { clicks = clicks + 1 }
}
'''.replace('MONITOR', '"' + args.screen + '"')
    scene.write_text(source, encoding='utf-8')
    scene.with_suffix('.luau').write_text('log("Luau geometry fixture loaded")\n', encoding='utf-8')
    env = dict(os.environ, APPDATA=str(work / 'state'), PLEAMAR_SOCKET_DIR=f'geometry-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1')
    log_path = work / 'native.log'

    def ask(command):
        return subprocess.check_output([str(binary), '--say', 'geometry', command], env=env,
            encoding='utf-8', stderr=subprocess.DEVNULL, timeout=3, creationflags=subprocess.CREATE_NO_WINDOW).strip()

    def wait_clicks(value):
        deadline = time.monotonic() + 9
        while time.monotonic() < deadline and process.poll() is None:
            try:
                reply = ask('get clicks')
                # The pipe can be ready while DX12 is still creating the first surface.
                if reply == '? the render does not answer':
                    time.sleep(.05)
                    continue
                count = int(reply)
                assert count <= value, f'An old or inactive region accepted a click: {count} > {value}'
                if count == value: return
            except subprocess.SubprocessError:
                pass
            time.sleep(.05)
        raise AssertionError(f'Did not receive click {value}')

    mouse = '50,70@1800 down@2000 up@2200 50,70@3800 down@4000 up@4200 250,70@5800 down@6000 up@6200 250,70@7800 down@8000 up@8200 250,70@10800 down@11000 up@11200 250,170@12800 down@13000 up@13200'
    with log_path.open('w', encoding='utf-8') as log:
        process = subprocess.Popen([str(binary), '--scene', str(scene), '--no-hud', '--stall', '0',
            '--mouse', mouse, '--seconds', '15'], env=env, stdout=log, stderr=subprocess.STDOUT,
            creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            wait_clicks(1)
            ask('fact offset 200')
            time.sleep(2.7)
            assert int(ask('get clicks')) == 1, 'The old position accepted input'
            wait_clicks(2)
            ask('fact enabled false')
            time.sleep(2.7)
            assert int(ask('get clicks')) == 2, 'Inactive zone accepted input'
            ask('fact enabled true')
            # Same zone count/dependencies, but new constant position and group structure.
            scene.write_text(source.replace('40 + offset, 50', '40 + offset, 150').replace('z: offset', 'z: offset; blur: 0'), encoding='utf-8')
            time.sleep(2.8)
            assert int(ask('get clicks')) == 2, 'The old position survived hot reload'
            wait_clicks(3)
            process.wait(timeout=5)
            assert process.returncode == 0
        finally:
            if process.poll() is None:
                try: ask('quit'); process.wait(timeout=5)
                except subprocess.SubprocessError: process.kill(); process.wait(timeout=5)
    content = log_path.read_text(encoding='utf-8')
    surfaces = [line for line in content.splitlines() if line.startswith('render · surface')]
    assert surfaces and all(' on ' + args.screen + ' ·' in line for line in surfaces), surfaces
    assert content.count('render · scene:') >= 2, 'Hot reload was not observed'
    assert 'runtime error:' not in content and 'panicked at' not in content, content
    print('PASS: moved/inactive/reloaded zones accept exactly three scripted clicks; default Luau, native rendering on', args.screen)
    print('Evidence:', work)


if __name__ == '__main__':
    main()
