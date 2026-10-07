#!/usr/bin/env python3
"""Measure the Marea renderer on one output without running its desktop commands."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--marea', type=Path, required=True, help='Checkout with its generated marea-desktop.plm')
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--screen', required=True, help='Exact output name')
    parser.add_argument('--output', type=Path, required=True, help='New evidence directory')
    parser.add_argument('--seconds', type=int, default=20)
    parser.add_argument('--repeats', type=int, default=2)
    args = parser.parse_args()
    assert os.name == 'nt', 'Requires native Windows'
    marea, binary = args.marea.resolve(strict=True), args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixture = output / 'optimization.plm'
    source = (marea / 'marea-desktop.plm').read_text(encoding='utf-8')
    for folder in ['common', 'lang', 'shaders', 'assets', 'wardrobe']:
        source = source.replace('"' + folder + '/', '"' + (marea / folder).as_posix() + '/')
    source = re.sub(r'(?m)^        keyboard: .*$', '        keyboard: none', source)
    source = re.sub(r'(?m)^        reserve: .*$', '', source)
    for condition, value in [('swim.here > 0.01', 'true'),
            ('open or tray_open or menu_open or reel_open or searching', 'false'), ('tide.on', 'false'),
            ('adventuring and screen.index == adv_screen', 'false'), ('tuck > 0.01 and screen.index == nook_screen', 'false')]:
        assert source.count('open: ' + condition) == 1, 'Marea surface contract changed: ' + condition
        source = source.replace('open: ' + condition, 'open: ' + value)
    assert 'kind: window' not in source and 'keyboard: on_demand' not in source and 'keyboard: exclusive' not in source
    fixture.write_text(source, encoding='utf-8')
    fixture.with_suffix('.luau').write_text('''-- Owned renderer fixture: no desktop commands, hotkeys or user preferences.
fact.language = "spanish"
fact.needed = true
fact.skin = "classic"
fact.windows_initialized = true
''', encoding='utf-8')
    subprocess.run([sys.executable, str(marea / 'windows/measure-desktop.py'), '--binary', str(binary),
        '--scene', str(fixture), '--screen', args.screen, '--output', str(output / 'measurement'),
        '--seconds', str(args.seconds), '--repeats', str(args.repeats), '--skins', 'classic', '--states', 'false', 'true'],
        check=True, creationflags=subprocess.CREATE_NO_WINDOW)


if __name__ == '__main__':
    main()
