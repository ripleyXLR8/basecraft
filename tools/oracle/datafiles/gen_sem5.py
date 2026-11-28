"""sem5.spec: positions and counts with option 0x8000; Go to List of Records."""
import os
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "gen_common.py"), encoding="utf-8").read())

F = "file:bc_s5.txt"
raw("step 86 opt=20000")
setvar("$$e", '"|"')
path_step(190, F); log("create")
path_step(191, F, "$id"); log("open", "$id")
setvar("$d", '"abcdefghij"')
raw(f"step 192 opt=4005 arg=2 {V}"); raw("text 32 $d"); raw("calc 0 $id"); log("w")


def setpos(n, label, err=False):
    raw("step 195 opt=c000"); raw("calc 0 $id"); raw(f"calc 1 {n}")
    if err:
        log(label)
    id_step(194, target="$x", calcs=["$id"]); log(label + "pos", "$x")


def read(arg, n, label, show='Length ( $r ) & "/" & Code ( $r )', pos=False):
    setvar("$r", '"unset"')
    if n is None:
        raw(f"step 193 opt=4005 arg={arg} {V}")
    else:
        raw(f"step 193 opt=c005 arg={arg} {V}")
        raw(f"calc 1 {n}")
    raw("text 32 $r"); raw("calc 0 $id")
    log(label, show)
    if pos:
        id_step(194, target="$x", calcs=["$id"]); log("pos", "$x")


setpos(3, "set3")
read(2, 2, "r2", "$r", True)
setpos(100, "set100", True)
setpos(-2, "setneg", True)
setpos(10, "set10")
read(2, 1, "reof", '"[" & $r & "]"', True)
read(2, None, "reofall", '"[" & $r & "]"')
setpos(0, "set0")
read(2, 0, "r0", '"[" & $r & "]"')
read(2, 100, "rbig", "$r")
setpos(0, "set0")
read(1, 2, "r16")
setpos(1, "set1")
read(1, 1, "r16odd", pos=True)
setpos(0, "set0")
read(3, 4, "rbytes", 'GetContainerAttribute ( $r ; "fileSize" ) & "/" & GetContainerAttribute ( $r ; "filename" )')
setpos(0, "set0")
read(0, 2, "rarg0", "$r")
setpos(2, "set2")
setvar("$d", '"XY"')
raw(f"step 192 opt=4205 arg=2 {V}"); raw("text 32 $d"); raw("calc 0 $id"); log("wlf")
id_step(194, target="$x", calcs=["$id"]); log("pos", "$x")
setpos(0, "set0")
read(2, None, "rall", 'Length ( $r ) & "/" & Code ( $r )')
setvar("$d", '"q"')
raw(f"step 192 opt=4005 arg=0 {V}"); raw("text 32 $d"); raw("calc 0 $id"); log("warg0")
id_step(194, target="$x", calcs=["$id"]); log("pos", "$x")
raw(f"step 192 opt=4005 arg=3 {V}"); raw("text 32 $d"); raw("calc 0 $id"); log("wbytes")
id_step(194, target="$x", calcs=["$id"]); log("pos", "$x")
id_step(196, calcs=["$id"]); log("close")
path_step(197, F); log("del")
# Persistent data.
raw("step 238 opt=c004"); raw("text 0 bckey"); raw('calc 1 "v1"'); log("persist")
# Go to List of Records: the list at parameter 7 (as the XML export shows it).
raw("step 7"); setvar("$ids", "Get ( RecordID )")
raw("step 7"); setvar("$ids", '$ids & "¶" & Get ( RecordID )')
raw("step 7"); raw("step 75 opt=80")
log("ids", 'Substitute ( $ids ; "¶" ; "," ) & "/" & Get ( FoundCount )')
for o in (0x400a, 0x800a, 0xa, 0xc00a):
    raw(f"step 228 opt={o:x} arg=5 inline=020103"); raw("calc 7 $ids")
    log(f"golist{o:x}", 'Get ( FoundCount ) & "/" & Get ( RecordID )')
raw("step 228 opt=400a arg=1"); raw('calc 7 GetValue ( $ids ; 2 ) & "¶999¶" & GetValue ( $ids ; 1 )')
log("golist2", 'Get ( FoundCount ) & "/" & Get ( RecordID ) & "/" & Get ( LayoutName )')
raw("step 228 opt=400a arg=1"); raw('calc 7 "999"'); log("golistbad", 'Get ( FoundCount )')
raw("step 228 opt=400a arg=1"); raw('calc 7 ""'); log("golistempty", 'Get ( FoundCount )')
raw("step 228 opt=400a arg=1"); raw('calc 7 GetValue ( $ids ; 1 ) & "¶" & GetValue ( $ids ; 1 )'); log("golistdup", 'Get ( FoundCount )')
finish()
