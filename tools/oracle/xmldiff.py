"""Compare two Save-as-XML exports object by object.

usage: python xmldiff.py <a.xml> <b.xml>
Prints, for each object whose attributes or text changed, the object path
and the changed items. Objects are keyed by their catalog path and id.
"""
import sys
import xml.etree.ElementTree as ET

SKIP = {'UUID', 'timestamp', 'File', 'modifications'}


def index(path):
    out = {}
    def walk(e, key):
        for i, c in enumerate(e):
            k = f"{key}/{c.tag}" + (f"[{c.get('id')}]" if c.get('id') else f"#{i}")
            items = {a: v for a, v in c.attrib.items() if a not in SKIP}
            text = (c.text or '').strip()
            if text and c.tag not in SKIP:
                items['#text'] = text
            out[k] = items
            walk(c, k)
    walk(ET.parse(path).getroot(), '')
    return out


a, b = index(sys.argv[1]), index(sys.argv[2])
for k in sorted(set(a) | set(b)):
    if k not in a:
        print('+', k, b[k])
    elif k not in b:
        print('-', k, a[k])
    elif a[k] != b[k]:
        ch = {x: (a[k].get(x), b[k].get(x)) for x in set(a[k]) | set(b[k]) if a[k].get(x) != b[k].get(x)}
        print('~', k, ch)
