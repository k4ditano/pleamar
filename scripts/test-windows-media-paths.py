"""Exercise native folder resolution and PNG writes with isolated child environments."""
import argparse
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tests', type=Path, required=True, help='Built pleamar library test executable')
    args = parser.parse_args()
    binary = args.tests.resolve(strict=True)
    assert os.name == 'nt'
    listing = subprocess.check_output([str(binary), '--list'], text=True, encoding='utf-8', timeout=30,
                                      creationflags=subprocess.CREATE_NO_WINDOW)
    for name in (None, 'My captures ñ 海', 'CON.txt', '../not-a-folder'):
        env = dict(os.environ)
        if name is None: env.pop('PLEAMAR_MEDIA_NAME', None)
        else: env['PLEAMAR_MEDIA_NAME'] = name
        tests = ['platform::windows_media_paths::tests::configured_media_directory_uses_native_known_folders']
        if name is None or name == 'My captures ñ 海':
            tests.append('platform::windows_capture::tests::unicode_png_roundtrip_does_not_overwrite_existing_photos')
        for test in tests:
            assert test + ': test' in listing.splitlines(), f'Missing native test: {test}'
            subprocess.run([str(binary), '--exact', test], env=env, check=True, timeout=30,
                           creationflags=subprocess.CREATE_NO_WINDOW | subprocess.BELOW_NORMAL_PRIORITY_CLASS)
    print('PASS: default/Unicode native media folders, invalid folder rejection and PNG round trips')


if __name__ == '__main__': main()
