"""Script snippet for validation_check.ps1: validation codes after a script's
commit in a copy of the Tasks template.
argv: out.xml, then `var:fieldid:name:calc` cases (each a new record, Set
Field, Commit, Get ( LastError ), Revert). Writes the results into
Tasks::Description, ending with END."""
import sys

def step(id_, name, body=""):
    return f'<Step enable="True" id="{id_}" name="{name}">{body}</Step>'

def f(id_, name):
    return f'<Field table="Tasks" id="{id_}" name="{name}"/>'

def setvar(name, calc):
    return step(141, "Set Variable",
                f"<Value><Calculation><![CDATA[{calc}]]></Calculation></Value>"
                f"<Repetition><Calculation><![CDATA[1]]></Calculation></Repetition><Name>{name}</Name>")

def setfield(fid, name, calc):
    return step(76, "Set Field", f"<Calculation><![CDATA[{calc}]]></Calculation>{f(fid, name)}")

COMMIT = step(75, "Commit Records/Requests", '<NoInteract state="True"/><Option state="False"/><ESSForceCommit state="False"/>')
REVERT = step(51, "Revert Record/Request", '<NoInteract state="True"/>')
NEW = step(7, "New Record/Request")

steps = [step(86, "Set Error Capture", '<Set state="True"/>')]
out = '""'
for case in sys.argv[2:]:
    # `?var:calc` records a calculation; `!sort:fid:name` sorts the found
    # set by a field, `!unsort` unsorts it, `!first` goes to the first record.
    if case.startswith("?"):
        var, calc = case[1:].split(":", 1)
        steps.append(setvar("$" + var, calc))
        out += f' & " {var}=" & ${var}'
        continue
    if case.startswith("!"):
        cmd = case[1:].split(":")
        if cmd[0] == "sort":
            steps.append(step(39, "Sort Records", '<NoInteract state="True"/><Restore state="True"/>'
                              '<SortList Maintain="True" value="True"><Sort type="Ascending"><PrimaryField>'
                              + f(int(cmd[1]), cmd[2]) + "</PrimaryField></Sort></SortList>"))
        elif cmd[0] == "unsort":
            steps.append(step(21, "Unsort Records"))
        elif cmd[0] in ("first", "next"):
            where = cmd[0].capitalize()
            steps.append(step(16, "Go to Record/Request/Page",
                              f'<NoInteract state="True"/><Exit state="False"/><RowPageLocation value="{where}"/>'))
        continue
    # var:fid:name:calc[|fid:name:calc...]; a var ending in + keeps the record.
    var, rest = case.split(":", 1)
    keep = var.endswith("+")
    var = var.rstrip("+")
    steps.append(NEW)
    for part in rest.split("|"):
        fid, name, calc = part.split(":", 2)
        steps.append(setfield(int(fid), name, calc))
    steps += [COMMIT, setvar("$" + var, "Get ( LastError )")]
    if not keep:
        steps.append(REVERT)
    out += f' & " {var}=" & ${var}'
steps += [NEW, setfield(3, "Description", out + ' & " END"'), COMMIT]
open(sys.argv[1], "w", encoding="utf-8").write('<fmxmlsnippet type="FMObjectList">' + "".join(steps) + "</fmxmlsnippet>")
