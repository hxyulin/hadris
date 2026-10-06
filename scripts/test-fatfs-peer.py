#!/usr/bin/env python3
"""Exercise the standalone helper's flat-fixture and failure contracts."""
import argparse
import subprocess
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('helper', type=Path)
    args = parser.parse_args()
    helper = args.helper.resolve()
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / 'source'
        source.mkdir()
        expected = {'EMPTY.TXT': b'', 'SMALL.TXT': b'fixture',
                    'PAYLOAD.BIN': bytes((i * 17 + i // 251) % 256 for i in range(131072))}
        for name, contents in expected.items():
            (source / name).write_bytes(contents)
        for bits, size in [(12, 2 << 20), (16, 16 << 20), (32, 64 << 20)]:
            image = root / f'{bits}.img'
            def run(workload, destination=source):
                result = subprocess.run([str(helper), workload, str(image), str(bits), str(size), str(destination)], capture_output=True, check=True)
                counters = result.stderr.decode().strip().split(',')
                assert len(counters) == 8 and counters[0] == 'IO' and counters[-2] == '0', counters
                return result
            run('format-empty')
            assert not run('list').stdout
            run('create-image')
            before = image.read_bytes()
            assert sorted(run('list').stdout.decode().splitlines()) == sorted('/' + name for name in expected)
            destination = root / f'extracted-{bits}'
            run('extract-tree', destination)
            assert {path.name: path.read_bytes() for path in destination.iterdir()} == expected
            assert image.read_bytes() == before, 'read workflow changed the image'
        image.write_bytes(b'bad image')
        for workload in ['list', 'unknown-workload']:
            result = subprocess.run([str(helper), workload, str(image), '32', str(64 << 20), str(source)], capture_output=True)
            assert result.returncode != 0
    print('ChaN helper: FAT12/16/32 empty/create/list/extract, read-only preservation and invalid inputs passed')


if __name__ == '__main__':
    main()
