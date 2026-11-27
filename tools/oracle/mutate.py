"""Write mutated copies of an .fmp12 file for reverse-oracle experiments.

usage: python mutate.py <src.fmp12> <plan.json> <outdir>
plan.json: {"batch-name": [[offset, byte_index, xor_mask], ...], ...}
Each batch becomes <outdir>/<batch-name>.fmp12 with every byte at
offset+byte_index XOR-ed with xor_mask. Sizes never change, so the
B-tree stays valid.
"""
import json, sys, os

src, plan, out = sys.argv[1:4]
data = open(src, 'rb').read()
for name, muts in json.load(open(plan)).items():
    b = bytearray(data)
    for off, i, mask in muts:
        b[off + i] ^= mask
    open(os.path.join(out, name + '.fmp12'), 'wb').write(b)
    print(name, len(muts))
