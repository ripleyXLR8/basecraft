import os, sys
out = []
V = "inline=0201000000000201200000000001"


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


def finish():
    raw("step 6 opt=a arg=5 inline=020103")
    raw("step 7")
    raw("step 76 opt=4001 inline=0403d000020002010300ea6b0001"); raw("calc 0 $$e")
    raw("step 75 opt=80")
    name = os.path.splitext(os.path.basename(sys.argv[0]))[0].replace("gen_", "")
    n = sum(1 for l in out if l.startswith("step "))
    assert n < 128, n
    open(os.path.join(os.path.dirname(os.path.abspath(sys.argv[0])), name + ".spec"), "w", encoding="utf-8", newline="\n").write("\n".join(out) + "\n")
    print(name, n, "steps")
