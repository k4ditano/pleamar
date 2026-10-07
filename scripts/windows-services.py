"""Read-only native desktop/service/hot-reload regression test; Windows only."""
from pathlib import Path
import argparse, json, os, shutil, subprocess, tempfile, time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', required=True, type=Path)
args = parser.parse_args()
binary = str(args.binary.resolve())
root = Path(__file__).resolve().parents[1]
env = dict(os.environ, PLEAMAR_SOCKET_DIR=f'services-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1')
with tempfile.TemporaryDirectory(prefix='pleamar services ñ ') as tmp:
    scene = Path(tmp) / 'native-services.plm'
    logic = scene.with_suffix('.luau')
    for path in [scene, logic]: shutil.copyfile(root/'tests'/path.name, path)
    with open(Path(tmp)/'run.log', 'w', encoding='utf-8') as log:
        process = subprocess.Popen([binary, '--scene', str(scene), '--no-hud', '--stall', '0'], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
        def ask(command):
            return subprocess.check_output([binary, '--say', scene.stem, command], env=env, stderr=subprocess.DEVNULL, text=True, timeout=10).strip()
        def ready():
            deadline = time.monotonic()+15
            while time.monotonic() < deadline:
                try:
                    if ask('get ready') == 'true': return
                except subprocess.SubprocessError: pass
                if process.poll() is not None: break
                time.sleep(.1)
            raise AssertionError((Path(tmp)/'run.log').read_text(encoding='utf-8'))
        def threads():
            return int(subprocess.check_output(['powershell.exe', '-NoProfile', '-Command', f'(Get-Process -Id {process.pid}).Threads.Count'], text=True))
        try:
            ready()
            time.sleep(2)
            before = threads()
            for k in range(5):
                ask('fact ready false')
                with logic.open('a', encoding='utf-8') as f: f.write(f'\n-- reload {k}\n')
                ready()
            time.sleep(2)
            after = threads()
            assert after <= before + 1, f'native subscriptions leaked threads: {before} -> {after}'
            assert ask('get media') == 'true'
            assert ask('get network') == 'true'
            for service in ['wifi', 'bluetooth', 'brightness', 'async_error', 'async_denied', 'async_query', 'async_query_error', 'async_query_denied', 'wallpaper', 'wallpaper_preview']:
                assert ask(f'get {service}') == 'true', f'{service} did not return native capability data'
            assert int(ask('get apps')) > 0
            print(json.dumps(dict(outputs=ask('get outputs'), apps=ask('get apps'), threads_before=before, threads_after=after, reloads=5)))
        finally:
            if process.poll() is None:
                ask('quit')
                process.wait(timeout=15)
