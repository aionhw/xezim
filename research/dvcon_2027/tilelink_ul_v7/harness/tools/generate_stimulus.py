#!/usr/bin/env python3
"""Deterministic legal TileLink-UL profile stimuli; no benchmark answer keys."""
import argparse, hashlib, json, random
from pathlib import Path

OPS = {"PutFullData": 0, "PutPartialData": 1, "Get": 4}

def make_transactions(seed: int, nreq: int):
    rng = random.Random(seed)
    txs = []
    masks = [1, 2, 3, 5, 8, 10, 12, 14]
    for i in range(nreq):
        # Repeated addresses make masked writes interact with later reads.
        # After the initial 16 writes, reads and writes interleave.
        op = 0 if i < 16 else (4 if i % 3 == 0 else 1 if i % 3 == 1 else 4)
        if i % 19 == 0 and i >= 16:
            op = 0
        src = (i * 7 + seed % 16) % 16
        word = (i * 5 + (i // 16) * 7) % 32
        if i >= 16 and i % 3 == 0:
            word = ((i - 16) * 5) % 32
        addr = word * 4
        mask = 15 if op != 1 else rng.choice(masks)
        data = rng.getrandbits(32)
        if i % 17 == 0:
            data = 0  # zero data with nonzero strobe catches truthiness errors
        txs.append({"i": i, "op": op, "source": src, "size": 2,
                    "address": addr, "mask": mask, "data": data})
    return txs

def pack(t):
    return (t['op'] << 61 | t['source'] << 57 | t['size'] << 54 |
            t['address'] << 38 | t['mask'] << 34 | t['data'] << 2)

def write(seed: int, nreq: int, out: Path):
    out.mkdir(parents=True, exist_ok=True)
    txs = make_transactions(seed, nreq)
    hex_path = out / 'stimulus.hex'
    hex_path.write_text(''.join(f'{pack(t):016x}\n' for t in txs))
    meta = {"format": "TLUL64_VECTOR_V1", "seed": seed, "count": nreq,
            "stimulus_sha256": hashlib.sha256(hex_path.read_bytes()).hexdigest(),
            "transactions": txs}
    (out / 'stimulus.json').write_text(json.dumps(meta, indent=2) + '\n')
    return meta

if __name__ == '__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--seed', type=int, required=True)
    p.add_argument('--count', type=int, default=96)
    p.add_argument('--out', type=Path, required=True)
    a=p.parse_args()
    if not 16 <= a.count <= 512:
        p.error('count must be in 16..512')
    print(json.dumps({k:v for k,v in write(a.seed,a.count,a.out).items() if k!='transactions'},indent=2))
