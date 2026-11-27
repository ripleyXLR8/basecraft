"""Reverse oracle for the script step catalogue.

usage:
  python step_ids.py mutate <raw.txt> <in.fmp12> <out.fmp12> <first id>
      overwrite the u16 step id (offset 20 of each 28-byte record of
      [17].[5].[S] key 4) of every step of every script with first, first+1…
      <raw.txt> is `fmdump <in.fmp12> raw`.
  python step_ids.py read <out.xml>...
      print `id<TAB>name` for every step of the exports (sorted, unique).
"""
import re
import sys
import xml.etree.ElementTree as ET


def step_lists(raw):
    for line in open(raw, encoding='utf-8'):
        f = line.rstrip('\n').split('\t')
        if len(f) == 5 and re.fullmatch(r'17\.5\.\d+', f[0]) and f[1] == 'kv' and f[2] == '4':
            yield int(f[4]), len(f[3]) // 2


def mutate(raw, src, dst, first):
    b = bytearray(open(src, 'rb').read())
    n = first
    for off, ln in step_lists(raw):
        for r in range(0, ln - 21, 28):
            b[off + r + 20:off + r + 22] = n.to_bytes(2, 'big')
            n += 1
    open(dst, 'wb').write(b)
    print(f'ids {first}..{n - 1}')


def read(paths):
    seen = {}
    for p in paths:
        for s in ET.parse(p).getroot().iter('Step'):
            if s.get('id') is not None and s.get('name') is not None:
                seen[int(s.get('id'))] = s.get('name')
    for k in sorted(seen):
        print(f'{k}\t{seen[k]}')


if __name__ == '__main__':
    if sys.argv[1] == 'mutate':
        mutate(sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5]))
    else:
        read(sys.argv[2:])
