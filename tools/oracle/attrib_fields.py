import json, subprocess, sys, re, collections
meta=json.load(open('meta.json'))
base='../fm/Contacts.xml'
tables=[130,131,132,133]
out=collections.defaultdict(list)
for name,muts in meta.items():
    diff=subprocess.run([sys.executable,'-I','C:/Users/richard/Documents/Claude Workspace/filemaker_pro_26/tools/oracle/xmldiff.py',base,name+'.xml'],capture_output=True,text=True).stdout.splitlines()
    for t,f,i,b in muts:
        ti=tables.index(t)
        hits=[]
        for l in diff:
            m=re.search(r'FieldsForTables#\d+/FieldCatalog#(\d+)/ObjectList#\d+/Field\[(\d+)\](\S*)',l)
            if m and int(m.group(2))==f and int(m.group(1))==ti:
                # FieldCatalog#k: child index inside FieldsForTables
                hits.append((m.group(1),m.group(3),l[:2]+l[l.index(' ',3):][:160]))
        # catalog child index -> table: compute from order
        out[(i,b)].append((t,f,[h for h in hits]))
for (i,b),v in sorted(out.items()):
    print(f'byte {i} bit {b}:')
    for t,f,h in v:
        cats=sorted({x[0] for x in h})
        print(f'   t{t} f{f} cats={cats}')
        for x in h: print('       ',x[1],x[2])
