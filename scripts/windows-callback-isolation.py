#!/usr/bin/env python3
"""Verify reply ownership when a plugin is inserted before a busy plugin; no display is selected."""
import argparse
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
    assert os.name == 'nt', 'This fixture runs native PowerShell helpers.'
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    scene = output / 'callback-isolation.plm'
    environment = dict(os.environ, APPDATA=str(output / 'state'),
                       PLEAMAR_SOCKET_DIR=f'callback-isolation-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1')
    report = dict(passed=False, graphical_validation=False, stages=[],
                  binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
    process, revision = None, 0

    def run(*arguments):
        result = subprocess.run([str(binary), *map(str, arguments)], env=environment, capture_output=True,
                                encoding='utf-8', timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode: raise RuntimeError(result.stdout + result.stderr)
        return result.stdout.strip()

    def ask(command): return run('--say', scene.stem, command)

    def until(predicate, label, seconds=10):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if process.poll() is not None: raise RuntimeError(f'Process exited during {label}')
            try:
                if predicate(): return
            except RuntimeError: pass
            time.sleep(.05)
        raise TimeoutError(label)

    def write_scene(names):
        nonlocal revision
        revision += 1
        imports = '\n'.join(f'import "{name}.plm"' for name in names)
        body = '\n'.join(names)
        source = f'{imports}\nscene Callbacks {{ surface {{ kind: window; size: 100, 100 }}\nfact revision_{revision} = true\n{body}\n}}'
        temporary = scene.with_suffix('.tmp')
        temporary.write_text(source, encoding='utf-8')
        temporary.replace(scene)
        if process: until(lambda: ask(f'get revision_{revision}') == 'true', f'scene revision {revision}')

    try:
        for name, delay, value in [('Existing', 5, 'existing'), ('Inserted', 0, 'inserted')]:
            (output / f'{name}.plm').write_text(f'''library {name} strict {{
                permissions {{ run: "powershell.exe" }}
                text result = "pending"
                fact replies = 0
                fact started = false
                component {name}() {{ size: 1, 1 }}
            }}''', encoding='utf-8')
            (output / f'{name}.luau').write_text(f'''run("powershell.exe", {{"-NoProfile", "-NonInteractive", "-Command",
                "Start-Sleep -Seconds {delay}; [Console]::WriteLine('{value}')"}}, function(value, code)
                    assert(code == 0)
                    text.result = value
                    fact.replies += 1
                end)
                fact.started = true
            ''', encoding='utf-8')
        scene.with_suffix('.luau').write_text('-- The parent uses Luau before any plugins arrive.\n', encoding='utf-8')
        write_scene(['Inserted', 'Existing'])
        # Only these generated, bounded stdout helpers are approved, in isolated APPDATA.
        (output / 'approval.log').write_text(run('--approve', scene, '--yes'), encoding='utf-8')
        write_scene([])
        with (output / 'native.log').open('w', encoding='utf-8') as log:
            process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen',
                f'pleamar-absent-callback-test-{os.getpid()}', '--no-hud', '--stall', '0', '--seconds', '60'],
                env=environment, stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
            until(lambda: ask(f'get revision_{revision}') == 'true', 'parent initialized')
            write_scene(['Existing'])
            until(lambda: ask('get Existing.started') == 'true', 'existing plugin starts its slow helper')
            report['stages'].append('first plugin has a pending native subprocess reply')
            write_scene(['Inserted', 'Existing'])
            until(lambda: ask('get Inserted.result') == 'inserted', 'new plugin receives its fast helper reply')
            report['existing_when_inserted_completes'] = ask('get Existing.result')
            assert report['existing_when_inserted_completes'] == 'pending', (
                'existing plugin consumed another reply, or the helpers did not overlap: ' + report['existing_when_inserted_completes'])
            until(lambda: ask('get Existing.result') == 'existing', 'existing plugin receives its own reply')
            report['replies'] = {name: ask(f'get {name}.replies') for name in ['Existing', 'Inserted']}
            assert all(value == '1' for value in report['replies'].values())
            report['stages'].append('both plugins receive exactly their own native helper output')
            ask('quit')
            report['exit_code'] = process.wait(timeout=10)
            assert report['exit_code'] == 0
        trace = (output / 'native.log').read_text(encoding='utf-8')
        assert 'waiting for one to appear' in trace and 'first frame' not in trace and 'panicked' not in trace
        report['passed'] = True
    except Exception as error:
        report['failure'] = str(error)
        raise
    finally:
        if process is not None and process.poll() is None:
            try: ask('quit'); process.wait(timeout=10)
            except Exception: process.kill(); process.wait(timeout=10)
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print('PASS: inserting a plugin preserves ownership of concurrent native subprocess replies; no graphical validation')


if __name__ == '__main__':
    main()
