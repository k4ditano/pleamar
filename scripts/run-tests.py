#!/usr/bin/env python3
"""The language, documentation and editor contracts, on every platform."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    os.chdir(ROOT)
    if args.binary is None:
        subprocess.run(['cargo', 'build', '--release', '--locked'], check=True)
    binary = (args.binary or ROOT / 'target/release' / ('pleamar.exe' if os.name == 'nt' else 'pleamar')).resolve()
    failures = []
    checks = 0

    def run(*args):
        return subprocess.run([str(binary), *map(str, args)], capture_output=True, encoding='utf-8', errors='replace', timeout=30)

    def check(ok, message):
        nonlocal checks
        checks += 1
        if not ok:
            failures.append(message)
            print('FAIL:', message)

    with tempfile.TemporaryDirectory(prefix='pleamar tests ñ ') as tmp:
        docs = []
        for note in ['README.md', 'docs/11-language-reference.md', 'docs/guide.md', 'docs/recipes.md']:
            for i, block in enumerate(re.findall(r'^```plm\r?\n(.*?)^```\s*$', Path(note).read_text(encoding='utf-8'), re.M | re.S)):
                path = Path(tmp) / f'{Path(note).stem}-{i}.plm'
                path.write_text(block, encoding='utf-8')
                docs.append(path)
        scenes = sorted(Path('tests').glob('*.plm')) + sorted(Path('examples').glob('*.plm')) + docs
        for path in scenes:
            first = path.read_text(encoding='utf-8').splitlines()[0]
            expected = first.removeprefix('// expect: ') if first.startswith('// expect: ') else 'ok'
            result = run('--check', path)
            output = result.stdout + result.stderr
            if expected == 'ok':
                ok = result.returncode == 0 and ': ok ·' in output
            elif expected.startswith('error «') and expected.endswith('»'):
                ok = result.returncode != 0 and expected[7:-1] in output
            else:
                raise ValueError(f'{path}: unknown expectation {expected}')
            check(ok, f'{path}: expected {expected}\n{output[:600]}')
        result = run('--grammar')
        grammar = result.stdout
        reference = Path('docs/11-language-reference.md').read_text(encoding='utf-8')
        written = ''.join(re.findall(r'^```vocabulary\n(.*?)^```\s*$', reference, re.M | re.S))
        check(result.returncode == 0 and grammar == written, 'reference vocabulary differs from compiler')
        vocab = dict(line.split(': ', 1) for line in grammar.splitlines())
        unexplained = set(vocab['statements'].split()) - set(vocab['documented'].split())
        check(not unexplained, f'statements without help: {sorted(unexplained)}')
        for editor, file in [('vim', 'editor/plm.vim'), ('vscode', 'editor/plm.tmLanguage.json')]:
            result = run('--highlight', editor)
            check(result.returncode == 0 and result.stdout == Path(file).read_text(encoding='utf-8'), f'{file}: outdated highlighting')
        corpus = '\n'.join(p.read_text(encoding='utf-8') for p in scenes + sorted(Path('tests/common').glob('*.plm')) + sorted(Path('examples/common').glob('*.plm')))
        unused = [f'{kind}/{word}' for kind, words in vocab.items() if kind not in ('language', 'units', 'documented') for word in words.split() if not re.search(r'(?<!\w)' + re.escape(word) + r'(?!\w)', corpus)]
        check(not unused, f'unused vocabulary: {unused}')
    print(f'language · {checks} checks, {len(failures)} failures ({len(docs)} documentation scenes)')
    return bool(failures)


if __name__ == '__main__':
    raise SystemExit(main())
