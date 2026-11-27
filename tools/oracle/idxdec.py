import sys
firsts={}
for l in open('firsts.txt'):
    i,p,k,key=l.rstrip('\n').split('\t'); firsts[int(i)]=(p,k,key)
b=open(sys.argv[1],'rb').read()
used=4076-int.from_bytes(b[4096+14:4096+16],'big')
p=b[4096+20:4096+20+used]
def pint(x):
    if len(x)==1: return x[0]
    if len(x)==2: return 0x80+((x[0]&0x7f)<<8)+x[1]
j=0; persist=[]; trans=[]
while j<len(p):
    raw=p[j]; pop= raw&0xc0==0xc0; op=raw&0x3f if pop else raw; j+=1
    ent=None
    if op in (0x20,0x28,0x30,0x38):
        if op==0x20: n= 8 if p[j]==0xfe else 1; j+= (1 if n==8 else 0)
        elif op==0x28: n=2
        elif op==0x30: n=3
        else: n=p[j]; j+=1
        e=p[j:j+n].hex(); j+=n
        if pop: trans.append(e)
        else: persist.append(e)
        continue
    if op in (0x3d,0x40): persist.pop(); continue
    if op==0x1b and p[j]==0: key=('1b',p[j+1]); val=p[j+2:j+6]; j+=6
    elif 1<=op<=5: key=('kv',p[j]); val=p[j+1:j+5]; j+=1+(1 if op==1 else 2*(op-1))
    elif 9<=op<=0xd: key=('kv',pint(p[j:j+2])); val=p[j+2:j+6]; j+=2+(1 if op==9 else 2*(op-9))
    elif 0x19<=op<=0x1d: k=p[j]; key=('kvl',p[j+1:j+1+k].hex()); val=p[j+1+k:j+5+k]; j+=1+k+(1 if op==0x19 else 2*(op-0x19))
    elif op==0: break
    else: print('??',hex(op)); break
    child=int.from_bytes(val,'big')
    ep='.'.join(persist+trans)
    fp=firsts.get(child)
    mark='' if fp and fp[0]==ep else '  <-- path differs'
    print(f'{ep:30} {key} -> {child:4} | first {fp}{mark}')
    trans=[]
    if pop and persist: persist.pop()
