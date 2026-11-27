import sys; sys.stdout.reconfigure(encoding='utf-8')
import json, os, xml.etree.ElementTree as ET
meta=json.load(open('tok/meta.json'))
res={}
for name,(t,muts) in sorted(meta.items()):
    x=f'tok/{name}.xml'
    if not os.path.exists(x): continue
    r=ET.parse(x).getroot()
    calc={}
    for fc in r.iter('FieldCatalog'):
        tid=fc.find('BaseTableReference').get('id')
        for f in fc.find('ObjectList'):
            c=[e.find('Text').text for e in f.iter('Calculation') if e.find('Text') is not None]
            calc[f'{tid}.3.5.{f.get("id")}.5']=c
    for p,off,v in muts:
        res[v]=calc.get(p)
json.dump(res,open('tok/result.json','w'))
for v in sorted(res): print(f'{v:02x}', [c[:50] for c in (res[v] or [])])
