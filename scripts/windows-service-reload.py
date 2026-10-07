#!/usr/bin/env python3
"""Exercise saved scene/service/Luau reloads through native Windows IPC, without opening a window."""
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
    parser.add_argument('--output', type=Path, required=True, help='new directory for the scene, log and report')
    args = parser.parse_args()
    assert os.name == 'nt', 'This rehearsal uses Windows named pipes and absent-output handling.'
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    scene = output / 'service-reload.plm'
    logic, log = scene.with_suffix('.luau'), output / 'native.log'
    environment = dict(os.environ, PLEAMAR_SOCKET_DIR=f'service-reload-{os.getpid()}',
                       PLEAMAR_NO_RELAUNCH='1', PLEAMAR_TIMING='1')
    report = dict(passed=False, graphical_validation=False, stages=[],
                  binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
    process, revision = None, 0

    def ask(command):
        reply = subprocess.run([str(binary), '--say', scene.stem, command], env=environment,
            capture_output=True, encoding='utf-8', timeout=5, creationflags=subprocess.CREATE_NO_WINDOW)
        if reply.returncode != 0:
            raise RuntimeError(reply.stdout + reply.stderr)
        return reply.stdout.strip()

    def until(predicate, label, seconds=5):
        deadline = time.monotonic() + seconds
        last = None
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f'Native process exited during {label}; see {log}')
            try:
                if predicate(): return
            except (RuntimeError, ValueError) as error:
                last = str(error)
            time.sleep(.05)
        raise TimeoutError(f'{label}: {last or "condition was not met"}')

    def write_scene(service='clock', year=False, permission=True):
        nonlocal revision
        revision += 1
        permissions = 'permissions { services: "clock" }' if permission else ''
        fields = 'second: number = -1' + ('; year: number = -1' if year else '')
        declaration = f'service {service} as now {{ {fields} }}' if service else ''
        source = f'''scene Reload {{
            surface {{ kind: window; size: 180, 100 }}
            {permissions}
            fact revision_{revision} = true
            fact heartbeat = 0
            fact logic_revision = 0
            {declaration}
        }}'''
        # Atomic replacement also exercises the directory watcher's save path.
        temporary = scene.with_suffix('.tmp')
        temporary.write_text(source, encoding='utf-8')
        temporary.replace(scene)
        if process:
            until(lambda: ask(f'get revision_{revision}') == 'true', f'scene revision {revision}')

    def write_logic(generation):
        logic.write_text(f'fact.logic_revision = {generation}\nevery(100, function() fact.heartbeat += 1 end)\n', encoding='utf-8')

    try:
        write_scene()
        write_logic(1)
        with log.open('w', encoding='utf-8') as stream:
            process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen',
                f'pleamar-absent-service-test-{os.getpid()}', '--no-hud', '--stall', '0', '--seconds', '90'],
                env=environment, stdout=stream, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
            until(lambda: float(ask('get now.second')) >= 0 and int(ask('get heartbeat')) >= 2, 'initial service and Luau')
            report['stages'].append('initial service and Luau')

            write_scene(year=True)
            until(lambda: float(ask('get now.year')) > 1900, 'new field in a quiet service', seconds=2)
            report['stages'].append('quiet snapshot fills added field')

            write_scene(service='clock.seconds')
            ask('fact now.second -2')
            until(lambda: float(ask('get now.second')) >= 0, 'replacement service ticks', seconds=3)
            report['stages'].append('same alias bound to a new source')

            write_scene(service='clock.seconds', permission=False)
            ask('fact now.second -3')
            until(lambda: ask('get now.second') == '-3', 'revoked service marker')
            deadline = time.monotonic() + 1.5
            while time.monotonic() < deadline:
                assert ask('get now.second') == '-3', 'revoked service still changes the scene'
                time.sleep(.08)
            report['stages'].append('permission revocation stops updates')
            write_scene(service='clock.seconds')
            until(lambda: float(ask('get now.second')) >= 0, 'restored permission')
            report['stages'].append('restored permission resumes updates')

            write_scene(service=None)
            write_scene()
            until(lambda: float(ask('get now.second')) >= 0, 'removed and restored quiet service', seconds=2)
            report['stages'].append('removed declaration restored with cached state')
            for generation in range(2, 8):
                ask('fact now.second -4')
                write_logic(generation)
                until(lambda: ask('get logic_revision') == str(generation) and float(ask('get now.second')) >= 0,
                      f'Luau reload {generation}')
            report['stages'].append('six Luau reloads preserve current service data')
            ask('quit')
            report['exit_code'] = process.wait(timeout=10)
            assert report['exit_code'] == 0
        trace = log.read_text(encoding='utf-8')
        assert 'waiting for one to appear' in trace and 'first frame' not in trace
        assert 'panicked' not in trace and 'initialization failed' not in trace
        report['passed'] = True
    except Exception as error:
        report['failure'] = str(error)
        # Keep the last native state available even when CI stops at this step.
        if log.exists(): print(log.read_text(encoding='utf-8', errors='replace'), flush=True)
        raise
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print('PASS: native IPC, saved scene/service/permission changes and six Luau reloads; no graphical validation')


if __name__ == '__main__':
    main()
