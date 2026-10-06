#!/usr/bin/env python3
"""Build the pinned ChaN FatFs host benchmark helper without installing tools."""
import argparse
import hashlib
import re
import subprocess
import urllib.request
import zipfile
from pathlib import Path

RESOURCES = {
    'arc/ff16.zip': '99f7dc1f7e095356e4a9e3dbe29959090d8b948afe2bbc5441e52fdf4b85449e',
    'patch/ff16p1.diff': '7996ddc3135f3d534a8153f7d75d587030e5c8551b003c3b7ee823ffb7fc64bf',
    'patch/ff16p2.diff': '5cd39f1fc299f0f1dec9fa1ce544dc0bc048b2f6eb3b8161476ed20f9cf1e290',
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('tests/target/chan-fatfs'))
    args = parser.parse_args()
    target = args.output.resolve()
    target.mkdir(parents=True, exist_ok=True)
    for resource, digest in RESOURCES.items():
        path = target / Path(resource).name
        if not path.exists():
            path.write_bytes(urllib.request.urlopen('https://elm-chan.org/fsw/ff/' + resource, timeout=60).read())
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise SystemExit(f'Checksum mismatch: {path}')
    with zipfile.ZipFile(target / 'ff16.zip') as archive:
        for name in ['ff.c', 'ff.h', 'ffconf.h', 'diskio.h', 'ffunicode.c']:
            (target / name).write_bytes(archive.read('source/' + name).replace(b'\r\n', b'\n'))
    for patch in ['ff16p1.diff', 'ff16p2.diff']:
        subprocess.run(['patch', '--batch', str(target / 'ff.c'), str(target / patch)], check=True)
    config = (target / 'ffconf.h').read_text()
    for name, value in {'FF_USE_MKFS': 1, 'FF_USE_LABEL': 1, 'FF_CODE_PAGE': 437,
                        'FF_USE_LFN': 3, 'FF_LFN_UNICODE': 2, 'FF_LFN_BUF': 765,
                        'FF_FS_EXFAT': 1, 'FF_LBA64': 1, 'FF_FS_NORTC': 1}.items():
        config, count = re.subn(r'(#define\s+' + name + r'\s+)\d+', rf'\g<1>{value}', config)
        assert count == 1, name
    (target / 'ffconf.h').write_text(config)
    root = Path(__file__).resolve().parent.parent
    subprocess.run(['cc', '-O2', '-std=c11', '-Wall', '-Wextra', '-Werror', '-I', str(target),
                    str(root / 'tests/peers/chan-fatfs.c'), str(target / 'ff.c'),
                    str(target / 'ffunicode.c'), '-o', str(target / 'chan-fatfs')], check=True)
    print(target / 'chan-fatfs')

if __name__ == '__main__':
    main()
