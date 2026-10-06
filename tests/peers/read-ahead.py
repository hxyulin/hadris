#!/usr/bin/env python3
"""Measure bounded read-ahead on FAT extraction and synthetic access patterns."""
import argparse
import csv
import json
import os
from pathlib import Path
import re
import random
import hashlib
import platform
import shutil
import statistics
import struct
import subprocess
import time


def fragment(source, destination):
    data = bytearray(source.read_bytes())
    sector = struct.unpack_from('<H', data, 11)[0]
    cluster_bytes = sector * data[13]
    reserved = struct.unpack_from('<H', data, 14)[0]
    fats = data[16]
    fat_sectors = struct.unpack_from('<I', data, 36)[0]
    fat_start = reserved * sector
    data_start = (reserved + fats * fat_sectors) * sector

    def location(cluster):
        return data_start + (cluster - 2) * cluster_bytes

    def next_cluster(cluster):
        return struct.unpack_from('<I', data, fat_start + cluster * 4)[0] & 0x0fffffff

    def set_link(cluster, link):
        for copy in range(fats):
            struct.pack_into('<I', data, fat_start + copy * fat_sectors * sector + cluster * 4, link)

    root = struct.unpack_from('<I', data, 44)[0]
    entry = None
    while root < 0x0ffffff8:
        for offset in range(location(root), location(root) + cluster_bytes, 32):
            if data[offset:offset + 11] == b'PAYLOAD BIN':
                entry = offset
                break
        if entry is not None:
            break
        root = next_cluster(root)
    assert entry is not None
    first = (struct.unpack_from('<H', data, entry + 20)[0] << 16) | struct.unpack_from('<H', data, entry + 26)[0]
    chain = []
    while first < 0x0ffffff8:
        assert first >= 2 and first not in chain
        chain.append(first)
        first = next_cluster(first)
    target = [10000 + i * 17 for i in range(len(chain))]
    assert all(next_cluster(c) == 0 and location(c) + cluster_bytes <= len(data) for c in target)
    chunks = [bytes(data[location(c):location(c) + cluster_bytes]) for c in chain]
    for c in chain:
        set_link(c, 0)
    for i, c in enumerate(target):
        data[location(c):location(c) + cluster_bytes] = chunks[i]
        set_link(c, target[i + 1] if i + 1 < len(target) else 0x0fffffff)
    struct.pack_into('<H', data, entry + 20, target[0] >> 16)
    struct.pack_into('<H', data, entry + 26, target[0] & 0xffff)
    destination.write_bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--image', type=Path, required=True, help='Prepared 1000-file FAT32 peer image')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=21)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    worker = args.worker.resolve()
    image = args.image.resolve()
    fragmented = out / 'fragmented.img'
    fragment(image, fragmented)
    blocks = out / 'blocks.img'
    with blocks.open('wb') as file:
        for block in range(32768):
            file.write(bytes([block % 251]) * 512)
    expected = {f'F{i:07}.TXT': b'fixture' for i in range(1000)}
    expected['PAYLOAD.BIN'] = bytes((i * 17 + i // 251) % 256 for i in range(131072))
    for fixture in [image, fragmented]:
        env = dict(os.environ, HADRIS_TESTS_PEER_WORKER='validate', HADRIS_TESTS_PEER_IMAGE=str(fixture), HADRIS_TESTS_PERF_FILES='1000')
        subprocess.run([str(worker)], env=env, check=True, capture_output=True)
    configs = [(layout, peer, capacity) for layout in ['contiguous', 'fragmented'] for peer in ['hadris/lazy', 'hadris/stream'] for capacity in [0, 16, 128]]
    (out / 'metadata.json').write_text(json.dumps(dict(
        worker_sha256=hashlib.sha256(worker.read_bytes()).hexdigest(),
        fixture_sha256=hashlib.sha256(image.read_bytes()).hexdigest(),
        fragmented_sha256=hashlib.sha256(fragmented.read_bytes()).hexdigest(),
        host=platform.platform(), samples_per_extraction_case=args.samples * 2,
        samples_per_pattern_case=args.samples, instrumented=False,
        extraction_order="deterministically shuffled each round",
    ), indent=2) + '\n')
    rows = []
    for trial in range(2):
        for sample in range(args.samples + 1):
            order = list(configs)
            random.Random(trial * 1000 + sample).shuffle(order)
            for layout, peer, capacity in order:
                fixture = image if layout == 'contiguous' else fragmented
                destination = out / 'extracted'
                assert not destination.exists()
                env = dict(os.environ, HADRIS_TESTS_PEER_WORKER=peer, HADRIS_TESTS_PEER_IMAGE=str(fixture), HADRIS_TESTS_PEER_DESTINATION=str(destination), HADRIS_TESTS_PEER_CACHE='hint', HADRIS_TESTS_PEER_READ_AHEAD_BLOCKS=str(capacity))
                start = time.perf_counter_ns()
                result = subprocess.run(['/usr/bin/time', '-l', str(worker)], env=env, text=True, capture_output=True)
                elapsed = time.perf_counter_ns() - start
                assert result.returncode == 0, result.stderr
                (out / f'{layout}-{peer.replace("/", "-")}-{capacity}-{trial}-{sample}.log').write_text(result.stderr)
                operation, reads, read_bytes, writes, failures, _ = map(int, re.search(r'WORKER_SAMPLE ns=(\d+) reads=(\d+) read_bytes=(\d+) writes=(\d+) failures=(\d+) driver_bytes=(\d+)', result.stderr).groups())
                rss = int(re.search(r'(\d+)\s+maximum resident set size', result.stderr)[1])
                assert writes == failures == 0
                assert {p.name: p.read_bytes() for p in destination.iterdir()} == expected
                shutil.rmtree(destination)
                if sample:
                    rows.append(dict(layout=layout, peer=peer, capacity_blocks=capacity, trial=trial + 1, sample=sample, elapsed_ns=elapsed, operation_ns=operation, peak_rss_bytes=rss, read_calls=reads, read_bytes=read_bytes))
        for layout, peer, capacity in configs:
            selected = [r for r in rows if (r['layout'], r['peer'], r['capacity_blocks'], r['trial']) == (layout, peer, capacity, trial + 1)]
            print(layout, peer, capacity, statistics.median(r['elapsed_ns'] for r in selected) / 1e6, selected[0]["read_calls"], selected[0]["read_bytes"], flush=True)
            (out / 'extraction.json').write_text(json.dumps(rows, indent=2) + '\n')
    patterns = []
    for pattern in ['sequential', 'alternating', 'fragmented', 'random']:
        for capacity in [0, 16, 128]:
            for sample in range(args.samples + 1):
                env = dict(os.environ, HADRIS_TESTS_PEER_WORKER=f'blocks/{pattern}', HADRIS_TESTS_PEER_IMAGE=str(blocks), HADRIS_TESTS_PEER_READ_AHEAD_BLOCKS=str(capacity))
                result = subprocess.run([str(worker)], env=env, text=True, capture_output=True)
                assert result.returncode == 0, result.stderr
                ns, reads, read_bytes = map(int, re.search(r'BLOCK_SAMPLE ns=(\d+) reads=(\d+) read_bytes=(\d+)', result.stderr).groups())
                if sample:
                    patterns.append(dict(pattern=pattern, capacity_blocks=capacity, sample=sample, elapsed_ns=ns, read_calls=reads, read_bytes=read_bytes))
            selected = [r for r in patterns if (r['pattern'], r['capacity_blocks']) == (pattern, capacity)]
            print(pattern, capacity, statistics.median(r['elapsed_ns'] for r in selected) / 1e6, selected[0]["read_calls"], selected[0]["read_bytes"], flush=True)
            (out / 'patterns.json').write_text(json.dumps(patterns, indent=2) + '\n')
    for name, samples in [('extraction', rows), ('patterns', patterns)]:
        with (out / f'{name}.csv').open('w', newline='') as file:
            writer = csv.DictWriter(file, fieldnames=list(samples[0]))
            writer.writeheader()
            writer.writerows(samples)


if __name__ == '__main__':
    main()
