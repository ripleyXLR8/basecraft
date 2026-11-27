"""steps_dump.py raw.txt marker: prints each step of the script whose first
comment is `marker`: index, id, arg, options, inline bytes, parameter keys."""
import sys, re, collections

raw, marker = sys.argv[1], sys.argv[2]
lists = collections.defaultdict(bytes)
longl = collections.defaultdict(dict)
params = collections.defaultdict(list)  # (script, n) -> [(rest, kind, key, val)]
for line in open(raw, encoding='utf-8', errors='replace'):
    p = line.rstrip('\n').split('\t')
    if len(p) < 4:
        continue
    path, kind, key, val = p[0], p[1], p[2], p[3]
    m = re.fullmatch(r'17\.5\.(\d+)', path)
    if m and kind == 'kv' and key == '4':
        lists[m.group(1)] += bytes.fromhex(val)
    m = re.fullmatch(r'17\.5\.(\d+)\.4', path)
    if m and kind == 'kv' and key != '0':
        longl[m.group(1)][int(key)] = bytes.fromhex(val)
    m = re.fullmatch(r'17\.5\.(\d+)\.5\.(\d+)((?:\.[^\t]+)?)', path)
    if m:
        params[(m.group(1), int(m.group(2)))].append((m.group(3), kind, key, val))
for s, d in longl.items():
    lists[s] = b''.join(d[k] for k in sorted(d))

def txt(h):
    try:
        b = bytes(c ^ 0x5A for c in bytes.fromhex(h))
        if all(32 <= c < 127 for c in b) and b:
            return repr(b.decode())
    except ValueError:
        pass
    return h

for s, b in lists.items():
    ps = params.get((s, 1), [])
    if not any(marker in txt(v) for (_, _, _, v) in ps):
        continue
    recs = [b[i:i + 28].ljust(28, b'\0') for i in range(0, len(b), 28)]
    for i, r in enumerate(recs):
        sid = int.from_bytes(r[20:22], 'big', signed=True)
        arg = int.from_bytes(r[22:24], 'big')
        o = int.from_bytes(r[24:28], 'big')
        print(f'#{i} id={sid} arg={arg} opt={o:#x} inline={r[6:20].hex()}')
        for rest, kind, key, val in params.get((s, i + 1), []):
            if key in ('216', '217') or rest.endswith('.5') and key in ('4', '6'):
                continue
            print(f'    {rest or "."} {kind} {key} {txt(val)}')
