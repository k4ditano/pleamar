"""Opt-in real desktop recording. Writes local MP4s; never uploads captured data."""
import argparse
import ctypes
from ctypes import wintypes
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import time
import wave

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--inspector', type=Path)
parser.add_argument('--desktop', action='store_true', help='explicitly allow capturing the monitor and system audio')
parser.add_argument('--stop', choices=['normal', 'reload', 'exit', 'timed', 'cancel'], default='normal')
parser.add_argument('--seconds', type=float, default=5)
parser.add_argument('--tone', action='store_true', help='play a quiet test tone to verify loopback audio')
parser.add_argument('--report', type=Path, required=True)
args = parser.parse_args()
assert os.name == 'nt' and args.desktop, 'use --desktop on an interactive Windows desktop'
assert 2 <= args.seconds <= 30
binary = args.binary.resolve()
environment = dict(os.environ, PLEAMAR_SOCKET_DIR=f'recording-smoke-{os.getpid()}', PLEAMAR_NO_RELAUNCH='1')
args.report.parent.mkdir(parents=True, exist_ok=True)
log = args.report.with_suffix('.log')

def usage(process):
    class Memory(ctypes.Structure):
        _fields_ = [('cb', wintypes.DWORD), ('faults', wintypes.DWORD)] + [(name, ctypes.c_size_t) for name in ['peak', 'working', 'quota_peak_paged', 'quota_paged', 'quota_peak_nonpaged', 'quota_nonpaged', 'pagefile', 'peak_pagefile', 'private']]
    times = [wintypes.FILETIME() for _ in range(4)]
    handle = wintypes.HANDLE(process._handle)
    assert ctypes.windll.kernel32.GetProcessTimes(handle, *(ctypes.byref(v) for v in times))
    cpu = sum((v.dwHighDateTime << 32) | v.dwLowDateTime for v in times[2:]) / 10_000_000
    memory = Memory(); memory.cb = ctypes.sizeof(memory)
    assert ctypes.windll.psapi.GetProcessMemoryInfo(handle, ctypes.byref(memory), memory.cb)
    return cpu, memory.working / 1024**2, memory.private / 1024**2

with tempfile.TemporaryDirectory(prefix='pleamar recording ñ ') as directory:
    directory = Path(directory)
    for suffix in ['.plm', '.luau']:
        (directory / ('native-recording' + suffix)).write_bytes((ROOT / 'tests' / ('native-recording' + suffix)).read_bytes())
    with log.open('w', encoding='utf-8') as output:
        runtime = max(15, int(args.seconds) + 10) if args.stop == 'timed' else 90
        process = subprocess.Popen([str(binary), '--scene', str(directory / 'native-recording.plm'), '--no-hud', '--stall', '0', '--seconds', str(runtime)], env=environment, stdout=output, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
        def ask(command):
            value = subprocess.run([str(binary), '--say', 'native-recording', command], env=environment, capture_output=True, text=True, encoding='utf-8', timeout=8, creationflags=subprocess.CREATE_NO_WINDOW)
            assert value.returncode == 0, value.stdout + value.stderr
            return value.stdout.strip()
        def until(check, seconds=30):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if process.poll() is not None: raise AssertionError(log.read_text(encoding='utf-8'))
                try:
                    if check(): return
                except (AssertionError, ValueError): pass
                time.sleep(.1)
            raise AssertionError(log.read_text(encoding='utf-8'))
        try:
            until(lambda: 'first frame' in log.read_text(encoding='utf-8'))
            time.sleep(2)
            baseline = usage(process)
            time.sleep(2)
            baseline_cpu = (usage(process)[0] - baseline[0]) / 2 / os.cpu_count() * 100
            ask('emit begin')
            if args.stop == 'cancel':
                ask('emit stop')
                until(lambda: ask('get phase') in ['cancelled', 'saved', 'error'])
                phase = ask('get phase')
                assert phase in ['cancelled', 'saved'], ask('get problem')
                report = dict(stop='cancel', state=phase, frames=int(ask('get frames')), path=ask('get path'))
                ask('quit'); assert process.wait(timeout=30) == 0
                args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
                print(json.dumps(report, ensure_ascii=True))
                raise SystemExit(0)
            until(lambda: ask('get phase') in ['recording', 'error'])
            assert ask('get phase') == 'recording', ask('get problem')
            path = Path(ask('get path'))
            before_ticks = int(ask('get ticks'))
            began = time.monotonic()
            initial = usage(process)
            if args.tone:
                import winsound
                tone = directory / 'quiet loopback.wav'
                with wave.open(str(tone), 'wb') as wav:
                    wav.setparams((2, 2, 48_000, 0, 'NONE', 'not compressed'))
                    wav.writeframes(b''.join(struct.pack('<hh', v, v) for n in range(int(48_000 * args.seconds)) for v in [int(256 * math.sin(2 * math.pi * 440 * n / 48_000))]))
                winsound.PlaySound(str(tone), winsound.SND_FILENAME | winsound.SND_ASYNC)
            time.sleep(args.seconds)
            elapsed = time.monotonic() - began
            final = usage(process)
            ticks = int(ask('get ticks')) - before_ticks
            assert ticks > 30 * args.seconds, f'Luau stalled during recording: {ticks}'
            if args.stop == 'normal':
                ask('emit stop')
                until(lambda: ask('get phase') in ['saved', 'error'])
                assert ask('get phase') == 'saved', ask('get problem')
            elif args.stop == 'reload':
                logic = directory / 'native-recording.luau'
                logic.write_text(logic.read_text(encoding='utf-8') + '\ntext.phase = "reloaded"\n', encoding='utf-8')
                until(lambda: ask('get phase') == 'reloaded')
                time.sleep(2)
            if args.stop != 'timed': ask('quit')
            assert process.wait(timeout=30) == 0
            assert path.exists() and path.stat().st_size > 1024
            report = dict(stop=args.stop, path=str(path), file_bytes=path.stat().st_size, measured_seconds=elapsed, luau_ticks=ticks, binary=str(binary), logical_cpus=os.cpu_count(), baseline_cpu_percent_machine=baseline_cpu, recording_cpu_percent_machine=(final[0] - initial[0]) / elapsed / os.cpu_count() * 100, working_mib=final[1], private_mib=final[2])
            if args.inspector:
                decoded = subprocess.run([str(args.inspector.resolve()), str(path)], capture_output=True, text=True, encoding='utf-8', timeout=90, creationflags=subprocess.CREATE_NO_WINDOW)
                assert decoded.returncode == 0, decoded.stderr
                report['decoded'] = json.loads(decoded.stdout)
                video, audio = report['decoded']['video'], report['decoded']['audio']
                assert video['seconds'] >= args.seconds - .2
                assert abs(video['seconds'] - video['decoded_samples'] / 60) < .05
                assert abs(video['seconds'] - audio['seconds']) < .15
                assert video['nonzero_bytes'] > 0
                if args.tone: assert audio['nonzero_bytes'] > 0, 'loopback tone was not captured'
            args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
            print(json.dumps(report, ensure_ascii=True))
        finally:
            if args.tone:
                import winsound
                winsound.PlaySound(None, 0)
            if process.poll() is None:
                try: ask('quit'); process.wait(timeout=30)
                except Exception: process.kill(); process.wait()
