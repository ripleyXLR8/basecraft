import sys, re, collections
import xml.etree.ElementTree as ET
name=sys.argv[1]
raw=[l.rstrip('\n').split('\t') for l in open(name+'.raw')]
pos=collections.defaultdict(list)
for path,kind,key,val in raw:
    for m in re.finditer(r'(?=([0-9a-f]{32}))',val):
        pass
    if len(val)==32: pos[val].append((path,kind,key))
r=ET.parse(name+'.xml').getroot()
parent={c:p for p in r.iter() for c in p}
res=collections.Counter()
for u in r.iter('UUID'):
    h=(u.text or '').replace('-','').lower()
    owner=parent[u]
    chain=[]
    e=owner
    while e in parent and len(chain)<3:
        chain.append(e.tag); e=parent[e]
    label='/'.join(reversed(chain))+' '+str({k:v for k,v in owner.attrib.items() if k in('id','name')})
    for path,kind,key in pos.get(h,[]):
        gen=re.sub(r'\b\d{3,}\b','N',path)
        res[(' > '.join(reversed(chain)), gen, key)]+=1
for (lab,path,key),n in sorted(res.items(), key=lambda x:x[0][1]):
    print(f'{n:4} {path:24} key={key:5} {lab}')
