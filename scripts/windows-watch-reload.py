#!/usr/bin/env python3
"""Native sys.watch/plugin isolation and reload checks; uses only owned files and no display."""
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
    parser.add_argument('--case', choices=['all', 'ownership', 'approval'], default='all')
    args = parser.parse_args()
    assert os.name == 'nt', 'This rehearsal uses Windows IPC and absent-output handling.'
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    scene, library = output / 'watch-reload.plm', output / 'plugin.plm'
    logic, plugin_logic = scene.with_suffix('.luau'), library.with_suffix('.luau')
    state = output / 'state'
    environment = dict(os.environ, APPDATA=str(state), PLEAMAR_SOCKET_DIR=f'watch-reload-{os.getpid()}',
                       PLEAMAR_NO_RELAUNCH='1', PLEAMAR_TIMING='1')
    report = dict(passed=False, graphical_validation=False, case=args.case, stages=[],
                  binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
    process, revision = None, 0
    log = output / 'native.log'

    def run(*arguments):
        result = subprocess.run([str(binary), *map(str, arguments)], env=environment, capture_output=True,
                                encoding='utf-8', timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode: raise RuntimeError(result.stdout + result.stderr)
        return result.stdout.strip()

    def ask(command): return run('--say', scene.stem, command)

    def until(predicate, label):
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline:
            if process.poll() is not None: raise RuntimeError(f'Process exited during {label}; see {log}')
            try:
                if predicate(): return
            except (RuntimeError, ValueError): pass
            time.sleep(.05)
        raise TimeoutError(label)

    def write(path, contents):
        temporary = path.with_suffix('.tmp')
        temporary.write_text(contents, encoding='utf-8')
        temporary.replace(path)

    def data(owner, contents):
        path = state / 'pleamar' / owner / 'state.txt'
        path.parent.mkdir(parents=True, exist_ok=True)
        write(path, contents)

    def write_scene(permission=True, include_plugin=True):
        nonlocal revision
        revision += 1
        write(scene, f'''{'import "plugin.plm"' if include_plugin else ''}
            scene WatchReload {{
                surface {{ kind: window; size: 100, 100 }}
                {'permissions { services: "files" }' if permission else ''}
                fact revision_{revision} = true
                fact code = 0
                fact allowed = false
                fact messages = 0
                text received = "none"
                {'NativeWatch' if include_plugin else ''}
            }}''')
        if process: until(lambda: ask(f'get revision_{revision}') == 'true', f'scene revision {revision}')

    def write_logic(path, generation):
        write(path, f'''local ok, supported = pcall(sys.watch, "files:state.txt", function(value)
                text.received = value
                fact.messages += 1
            end)
            fact.allowed = ok and supported == true
            fact.code = {generation}
        ''')

    try:
        write(library, '''library NativeWatch strict {
            permissions { services: "files" }
            fact code = 0
            fact allowed = false
            fact messages = 0
            text received = "none"
            component NativeWatch() { size: 10, 10 }
        }''')
        write_logic(logic, 1)
        write_logic(plugin_logic, 1)
        write_scene()
        data(scene.stem, 'scene initial')
        data('NativeWatch', 'plugin initial')
        # Approval is confined to APPDATA in the new output directory. The only
        # plugin is our generated fixture, which can read only its owned files.
        approval = run('--approve', scene, '--yes')
        assert 'approved' in approval, approval
        (output / 'approval.log').write_text(approval, encoding='utf-8')
        with log.open('w', encoding='utf-8') as stream:
            process = subprocess.Popen([str(binary), '--scene', str(scene), '--screen',
                f'pleamar-absent-watch-test-{os.getpid()}', '--no-hud', '--stall', '0', '--seconds', '120'],
                env=environment, stdout=stream, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
            until(lambda: ask('get allowed') == 'true' and ask('get NativeWatch.allowed') == 'true', 'both watchers initialized')
            until(lambda: ask('get received') != 'none' and ask('get NativeWatch.received') != 'none', 'initial file snapshots')
            report['stages'].append('scene and approved plugin initialized')

            if args.case != 'approval':
                data(scene.stem, 'scene private update')
                until(lambda: ask('get received') == 'scene private update', 'scene receives its file')
                time.sleep(.3)
                report['plugin_after_scene_update'] = ask('get NativeWatch.received')
                assert report['plugin_after_scene_update'] == 'plugin initial', 'scene file reached the plugin'
                data('NativeWatch', 'plugin private update')
                until(lambda: ask('get NativeWatch.received') == 'plugin private update', 'plugin receives its file')
                assert ask('get received') == 'scene private update', 'plugin file reached the scene'
                report['stages'].append('actual scene/plugin file updates remain isolated')

                write_scene(permission=False)
                data(scene.stem, 'scene while denied')
                data('NativeWatch', 'plugin still allowed')
                until(lambda: ask('get NativeWatch.received') == 'plugin still allowed', 'plugin keeps its own permission')
                time.sleep(.4)
                assert ask('get received') == 'scene private update', 'revoked scene still received data'
                write_scene()
                until(lambda: ask('get received') == 'scene while denied', 'restored permission replays latest state')
                report['stages'].append('permission revocation/restoration preserves owner isolation')

                for generation in range(2, 8):
                    write_logic(logic, generation)
                    until(lambda: ask('get code') == str(generation) and ask('get allowed') == 'true', f'Luau generation {generation}')
                data(scene.stem, 'scene after reloads')
                data('NativeWatch', 'plugin after reloads')
                until(lambda: ask('get received') == 'scene after reloads' and ask('get NativeWatch.received') == 'plugin after reloads',
                      'both file subscriptions after six Lua reloads')
                report['stages'].append('six actual Lua reloads preserve isolated subscriptions')
                write_scene(include_plugin=False)
                data('NativeWatch', 'plugin while absent')
                time.sleep(.35)
                write_scene()
                until(lambda: ask('get NativeWatch.received') == 'plugin while absent', 'recreated plugin gets its current snapshot')
                report['stages'].append('removed and recreated plugin receives current private state')

            if args.case != 'ownership':
                write_logic(plugin_logic, 2)
                until(lambda: ask('get NativeWatch.code') == '2', 'changed plugin logic runs')
                report['changed_plugin_allowed'] = ask('get NativeWatch.allowed')
                assert report['changed_plugin_allowed'] == 'false', 'changed plugin retained approval for different code'
                data('NativeWatch', 'plugin pending approval')
                previous = ask('get NativeWatch.received')
                time.sleep(.4)
                assert ask('get NativeWatch.received') == previous, 'unapproved plugin received file updates'
                approval = run('--approve', scene, '--yes')
                (output / 'reapproval.log').write_text(approval, encoding='utf-8')
                write_logic(logic, 8)
                until(lambda: ask('get NativeWatch.allowed') == 'true' and ask('get NativeWatch.received') == 'plugin pending approval',
                      'reapproved plugin resumes on reload')
                report['stages'].append('changed plugin approval is revoked and explicit reapproval restores service access')

            ask('quit')
            report['exit_code'] = process.wait(timeout=10)
            assert report['exit_code'] == 0
        trace = log.read_text(encoding='utf-8')
        assert 'waiting for one to appear' in trace and 'first frame' not in trace and 'panicked' not in trace
        report['passed'] = True
    except Exception as error:
        report['failure'] = str(error)
        raise
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print('PASS: native file watchers, isolated plugin permissions and reloads; no graphical validation')


if __name__ == '__main__':
    main()
