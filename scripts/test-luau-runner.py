"""Check that isolated logic tests report failures and enforce their limits."""
from pathlib import Path
import argparse
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runner', type=Path, required=True)
args = parser.parse_args()
with tempfile.TemporaryDirectory(prefix='pleamar logic ñ ') as tmp:
    path = Path(tmp) / 'checks.luau'
    cases = [
        ('assert(sys == nil and run == nil and spawn == nil); log("PASS isolated Luau")', True, 'PASS isolated Luau'),
        ('error("deliberate failure")', False, 'deliberate failure'),
        ('local =', False, 'syntax error'),
        ('while true do end', False, 'exceeded two seconds'),
        ('local huge = string.rep("x", 128 * 1024 * 1024)', False, 'memory'),
    ]
    for source, success, marker in cases:
        path.write_text(source, encoding='utf-8')
        result = subprocess.run([str(args.runner.resolve()), str(path)], capture_output=True, text=True,
                                encoding='utf-8', timeout=15, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        assert (result.returncode == 0) == success and marker in result.stdout + result.stderr, result
    path.write_text('error("compile-only must not execute this")', encoding='utf-8')
    result = subprocess.run([str(args.runner.resolve()), '--compile-only', str(path)], capture_output=True, timeout=15,
                            creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    assert result.returncode == 0 and not result.stdout and not result.stderr, result
    print('PASS: isolated Luau success, runtime/syntax failures, execution and memory limits')
