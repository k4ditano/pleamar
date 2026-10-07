"""Exercise skill installation in an isolated Unicode home, without agent settings."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', required=True, type=Path)
args = parser.parse_args()
binary = args.binary.resolve()
with tempfile.TemporaryDirectory(prefix='pleamar skills ñ ') as directory:
    home = Path(directory)
    codex = home / '.codex'
    codex.mkdir()
    env = dict(os.environ, HOME=str(home), USERPROFILE=str(home),
               XDG_CONFIG_HOME=str(home / '.config'), CODEX_HOME=str(codex))
    if sys.platform == 'win32':
        env.pop('HOME', None)  # Normal native Windows account discovery.
    def install():
        result = subprocess.run([str(binary), '--install-skill'], env=env,
            capture_output=True, encoding='utf-8', errors='replace', timeout=20)
        assert result.returncode == 0, result.stdout + result.stderr
    install()
    skill = codex / 'skills/pleamar/SKILL.md'
    desktop = codex / 'skills/pleamar-desktop/SKILL.md'
    original = skill.read_text(encoding='utf-8')
    assert '<!-- pleamar skill ' in original
    assert (skill.parent / 'measuring.md').is_file()
    assert desktop.exists() == (sys.platform == 'linux')
    skill.write_text('<!-- pleamar skill obsolete. -->', encoding='utf-8')
    install()
    assert skill.read_text(encoding='utf-8') == original
    skill.write_text('User-owned skill — keep this.', encoding='utf-8')
    install()
    assert skill.read_text(encoding='utf-8') == 'User-owned skill — keep this.'
print('PASS: isolated skill install/refresh, native home discovery, user ownership and platform scope')
