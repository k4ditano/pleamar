"""Select between two owned native SMTC sessions on the requested monitor."""
import argparse
import ctypes
import os
from pathlib import Path
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--fixture', type=Path, required=True)
parser.add_argument('--screen', required=True)
args = parser.parse_args()
flags = subprocess.CREATE_NO_WINDOW
user32 = ctypes.WinDLL('user32')
user32.GetForegroundWindow.restype = ctypes.c_void_p
foreground = user32.GetForegroundWindow()
processes = []
with tempfile.TemporaryDirectory(prefix='pleamar media choice ñ ') as temporary:
    folder = Path(temporary)
    env = dict(os.environ, APPDATA=str(folder / 'state'), PLEAMAR_SOCKET_DIR='media-choice-' + str(os.getpid()))
    logs = []
    try:
        for index in range(2):
            log = (folder / f'player-{index}.log').open('w', encoding='utf-8'); logs.append(log)
            p = subprocess.Popen([str(args.fixture.resolve()), '--screen', args.screen], stdout=log, stderr=log, creationflags=flags)
            processes.append(p)
        first, second = ['org.pleamar.validation.media.' + str(p.pid) for p in processes]
        scene = folder / 'choice.plm'
        scene.write_text('''scene Choice {
            surface { size: 1, 1; keyboard: none }
            permissions { services: "media", "media.*" }
            fact selected = 0
            fact pinned = 0
            fact count = 0
            fact failed = false
            fact playing = false
            event first ->
            event second ->
            event auto ->
            event gone ->
            event pause_first ->
        }''', encoding='utf-8')
        scene.with_suffix('.luau').write_text('''
local first, second = "''' + first + '", "' + second + '''"
local function update(value)
    fact.selected = value.player == first and 1 or value.player == second and 2 or 0
    fact.playing = value.playing == true
    local count, pinned = 0, 0
    for _, p in ipairs(value.players or {}) do
        if p.id == first or p.id == second then
            count += 1
            if p.chosen then pinned += 1 end
        end
    end
    fact.count, fact.pinned = count, pinned
end
sys.watch("media", update)
local function choose(id)
    sys.call_async("media.choose", {id}, function(error, code)
        fact.failed = code ~= 0
        sys.ask_async("media.state", {}, function(value, error) if value then update(value) end end)
    end)
end
on("first", function() choose(first) end)
on("second", function() choose(second) end)
on("auto", function() choose("") end)
on("gone", function() choose(first .. "-gone") end)
on("pause_first", function()
    sys.call_async("media.pause", {first}, function(error, code) fact.failed = code ~= 0 end)
end)
''', encoding='utf-8')
        log = (folder / 'engine.log').open('w', encoding='utf-8'); logs.append(log)
        engine = subprocess.Popen([str(args.binary.resolve()), '--scene', str(scene), '--screen', args.screen,
            '--no-hud', '--stall', '0', '--seconds', '90'], env=env, stdout=log, stderr=log, creationflags=flags)
        processes.append(engine)
        def ask(command):
            return subprocess.check_output([str(args.binary.resolve()), '--say', 'choice', command], env=env,
                encoding='utf-8', stderr=subprocess.DEVNULL, timeout=4, creationflags=flags).strip()
        def wait(field, expected):
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                try:
                    value = ask('get ' + field)
                    if {'true': 1, 'false': 0}.get(value, value) == expected or value == str(expected): return
                except (subprocess.SubprocessError, ValueError): pass
                time.sleep(.1)
            raise AssertionError((field, ask('get ' + field), expected))
        wait('count', 2)
        ask('emit first'); wait('selected', 1); wait('pinned', 1)
        ask('emit pause_first'); wait('playing', 0)
        ask('emit second'); wait('selected', 2); wait('playing', 1)
        ask('emit gone'); wait('failed', 1); wait('selected', 2)
        ask('emit auto'); wait('pinned', 0)
        ask('emit first'); wait('selected', 1)
        processes[0].terminate(); processes[0].wait(timeout=5)
        wait('count', 1); wait('selected', 2); wait('pinned', 0)
        assert user32.GetForegroundWindow() == foreground, 'foreground changed'
        print('PASS: two native SMTC sessions, selected transport target, stale ID rejection, automatic reset, player close fallback; foreground unchanged')
        ask('quit'); engine.wait(timeout=10)
    finally:
        for p in reversed(processes):
            if p.poll() is None: p.terminate(); p.wait(timeout=10)
        for log in logs: log.close()
