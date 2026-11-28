import os

"""Builds sem1.spec: the data file / PDF / misc steps with Get ( LastError )
after each, results in $e, stored in Tasks::Description of a new record."""
out = []
V = "inline=0201000000000201200000000001"  # variable target slots


def raw(line):
    out.append(line)


def setvar(name, expr):
    raw("step 141 opt=4004")
    raw(f"text 0 {name}")
    raw(f"calc 1 {expr}")


def log(label, extra='""'):
    setvar("$$e", f'$$e & " {label}=" & Get ( LastError ) & ":" & {extra}')


def path_step(sid, path, target=None, opt=0x20):
    if target:
        raw(f"step {sid} opt={opt | 0x5:x} {V}")
        raw(f"text 32 {target}")
    else:
        raw(f"step {sid} opt={opt:x}")
    kind, p = path.split(":", 1) if ":" in path and not path.startswith("$") else ("", path)
    raw(f"path 0 {kind} {p}")


def id_step(sid, arg=0, target=None, calcs=(), opt=0x4000):
    if target:
        raw(f"step {sid} opt={opt | 0x5:x} arg={arg} {V}")
        raw(f"text 32 {target}")
    else:
        raw(f"step {sid} opt={opt:x} arg={arg}")
    for i, c in enumerate(calcs):
        if c is not None:
            raw(f"calc {i} {c}")


F = "file:bc_df.txt"
raw("step 86 opt=20000")  # Set Error Capture On
setvar("$$e", '"|"')
path_step(188, F, "$x"); log("gfe0", "$x")
path_step(189, F, "$x"); log("gfs0", "$x")
path_step(191, F, "$id"); log("open0", "$id")
path_step(190, F); log("create")
path_step(190, F); log("create2")
path_step(188, F, "$x"); log("gfe1", "$x")
path_step(189, F, "$x"); log("gfs1", "$x")
path_step(191, F, "$id"); log("open", "$id")
path_step(191, F, "$id2"); log("open2", "$id2")
setvar("$d", '"héllo"')
# Write: id, variable source $d, UTF-8 (arg 2), append line feed (0x200).
raw(f"step 192 opt=4205 arg=2 {V}"); raw("text 32 $d"); raw("calc 0 $id"); log("w8")
id_step(194, target="$x", calcs=["$id"]); log("pos", "$x")
setvar("$d", '"ab"')
raw(f"step 192 opt=4005 arg=1 {V}"); raw("text 32 $d"); raw("calc 0 $id"); log("w16")
id_step(194, target="$x", calcs=["$id"]); log("pos2", "$x")
setvar("$d", '"Z"')
raw(f"step 192 opt=4005 arg=3 {V}"); raw("text 32 $d"); raw("calc 0 $id"); log("wb")
id_step(194, target="$x", calcs=["$id"]); log("pos3", "$x")
id_step(196, calcs=["$id"]); log("close")
path_step(189, F, "$x"); log("size", "$x")
path_step(191, F, "$id"); log("reopen", "$id")
id_step(194, target="$x", calcs=["$id"]); log("pos0", "$x")
id_step(193, arg=2, target="$r", calcs=["$id", "3"]); log("r3", 'Length ( $r ) & "/" & Code ( $r )')
id_step(193, arg=2, target="$r", calcs=["$id", "2"]); log("r2", 'Length ( $r ) & "/" & Code ( $r )')
id_step(194, target="$x", calcs=["$id"]); log("pos5", "$x")
id_step(195, calcs=["$id", "0"]); log("set0")
id_step(193, arg=2, target="$r", calcs=["$id"]); log("rall", 'Length ( $r ) & "/" & Code ( $r )')
id_step(193, arg=2, target="$r", calcs=["$id", "1"]); log("reof", 'Length ( $r ) & "/" & Code ( $r )')
id_step(195, calcs=["$id", "100"]); log("set100")
id_step(194, target="$x", calcs=["$id"]); log("pos100", "$x")
id_step(195, calcs=["$id", "7"]); log("set7")
id_step(193, arg=1, target="$r", calcs=["$id", "2"]); log("r16", 'Length ( $r ) & "/" & Code ( $r )')
id_step(195, calcs=["$id", "0"]); log("set0b")
id_step(193, arg=3, target="$r", calcs=["$id", "2"]); log("rbytes", 'GetAsText ( $r ) & "/" & GetContainerAttribute ( $r ; "filename" )')
id_step(195, calcs=["$id", "-1"]); log("setneg")
id_step(196, calcs=["$id"]); log("close2")
path_step(190, F); log("createexisting")
path_step(189, F, "$x"); log("sizeaftercreate", "$x")
id_step(196, calcs=["$id"]); log("closeagain")
id_step(193, arg=2, target="$r", calcs=["$id"]); log("rclosed")
id_step(196, calcs=["12345"]); log("closebad")
id_step(193, arg=2, target="$r", calcs=['""']); log("rnoid")
raw("step 1 opt=40 inline=02010b")
raw("script BC Steps 2")
raw("step 86 opt=20000")
# Rename, delete.
raw("step 199 opt=4020"); raw("path 0 file bc_df.txt"); raw('calc 0 "bc_df2.txt"'); log("ren")
path_step(188, "file:bc_df2.txt", "$x"); log("gferen", "$x")
raw("step 199 opt=4020"); raw("path 0 file bc_nope.txt"); raw('calc 0 "bc_df3.txt"'); log("renmiss")
path_step(197, "file:bc_df2.txt"); log("del")
path_step(197, "file:bc_df2.txt"); log("delmiss")
path_step(190, "file:bc_sub/x.txt", opt=0x220); log("createdirs")
path_step(190, "file:bc_sub2/x.txt"); log("createnodirs")
path_step(188, "file:bc_sub", "$x"); log("gfedir", "$x")
path_step(189, "file:bc_sub", "$x"); log("gfsdir", "$x")
setvar("$p", '"file:bc_sub/x.txt"')
path_step(188, "$p", "$x"); log("gfevar", "$x")
path_step(197, "file:bc_sub/x.txt"); log("delsub")
path_step(188, "file:", "$x"); log("gfeempty", "$x")
# PDF documents.
raw("step 247"); log("cancel0")
raw("step 245 opt=20"); raw("path 0 file bc_out.pdf"); log("closepdf0")
raw("step 244 opt=20"); raw("path 0 file bc_in.pdf"); log("append0")
raw("step 243"); log("createpdf")
raw("step 243"); log("createpdf2")
raw("step 244 opt=20"); raw("path 0 file bc_in.pdf"); log("appendmiss")
raw("step 245 opt=20"); raw("path 0 file bc_out.pdf"); log("closepdf")
path_step(188, "file:bc_out.pdf", "$x"); log("pdfexists", "$x")
path_step(189, "file:bc_out.pdf", "$x"); log("pdfsize", "$x")
raw("step 246 opt=20"); raw("path 0 file bc_out.pdf"); log("openpdf")
raw("step 244 opt=20"); raw("path 0 file bc_out.pdf"); log("appendself")
raw("step 247"); log("cancel")
raw("step 242 opt=a0"); raw("path 0 file bc_nope.pdf"); log("printmiss")
path_step(197, "file:bc_out.pdf"); log("delpdf")
# Misc.
raw("step 238 opt=4004"); raw("text 0 bckey"); raw('calc 1 "v1"'); log("persist")
raw("step 238 opt=4000"); raw('calc 1 "v1"'); log("persistnoname")
raw("step 187"); log("notif")
raw("step 212"); log("ai")
raw("step 78"); log("insobj")
raw("step 158"); log("inspdf")
raw("step 159"); log("insav")
# Go to List of Records: record ids 1 and 2 on Task Details (layout 3).
raw("step 7"); raw("step 7"); raw("step 75 opt=80")
raw("step 228 opt=400a arg=5 inline=020103"); raw('calc 0 "1¶2"'); log("golist", 'Get ( FoundCount ) & "/" & Get ( LayoutName ) & "/" & Get ( RecordID )')
raw("step 228 opt=400a arg=5 inline=020103"); raw('calc 0 "999"'); log("golistbad", 'Get ( FoundCount )')
raw("step 228 opt=400a arg=5 inline=020103"); raw('calc 0 ""'); log("golistempty", 'Get ( FoundCount )')
raw("step 228 opt=4000 arg=1"); raw('calc 0 "2¶1¶2"'); log("golistorig", 'Get ( FoundCount ) & "/" & Get ( RecordID )')
# Results.
raw("step 6 opt=a arg=5 inline=020103")
raw("step 7")
raw("step 76 opt=4001 inline=0403d000020002010300ea6b0001"); raw("calc 0 $$e")
raw("step 75 opt=80")
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "sem1.spec"), "w", encoding="utf-8", newline="\n").write("\n".join(out) + "\n")
