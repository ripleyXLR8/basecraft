"""Builds an fmxmlsnippet script that evaluates calculation expressions.

    python calc_snippet.py exprs.txt snippet.xml

exprs.txt holds one expression per line (blank lines and lines starting with
`#` are skipped). The script evaluates each line with Evaluate() and writes
`index TAB result TAB EvaluationError` lines (result escaped: \\ \t \r \n)
into Tasks::Description of a new record. calc_oracle.ps1 runs it and reads
the field back through UI Automation.
"""
import sys


def fm_literal(s):
    # FileMaker text literal: \ and " are escaped, a bare pilcrow is a CR.
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("¶", "\\¶") + '"'


def step(id_, name, body=""):
    return f'<Step enable="True" id="{id_}" name="{name}">{body}</Step>'


def setvar(name, calc):
    return step(141, "Set Variable",
                f"<Value><Calculation><![CDATA[{calc}]]></Calculation></Value>"
                f"<Repetition><Calculation><![CDATA[1]]></Calculation></Repetition><Name>{name}</Name>")


def main():
    exprs = [l.rstrip("\r\n") for l in open(sys.argv[1], encoding="utf-8")]
    exprs = [e for e in exprs if e.strip() and not e.startswith("#")]
    # A calculation holds at most 30,000 characters: build $__src in chunks.
    chunks, cur = [], ""
    for e in exprs:
        part = fm_literal(e)[1:-1] + "¶"
        if cur and len(cur) + len(part) > 20000:
            chunks.append(cur)
            cur = ""
        cur += part
    chunks.append(cur)
    esc = 'Substitute ( TextFormatRemove ( $__r ) ; [ "\\\\" ; "\\\\\\\\" ] ; [ Char ( 9 ) ; "\\\\t" ] ; [ Char ( 13 ) ; "\\\\r" ] ; [ Char ( 10 ) ; "\\\\n" ] )'
    steps = [
        step(86, "Set Error Capture", '<Set state="True"/>'),
        setvar("$__src", '""'),
        *[setvar("$__src", f'$__src & "{c}"') for c in chunks],
        setvar("$__n", "ValueCount ( $__src )"),
        setvar("$__i", "0"),
        setvar("$__out", '""'),
        step(71, "Loop", '<FlushType value="Always"/>'),
        setvar("$__i", "$__i + 1"),
        step(72, "Exit Loop If", "<Calculation><![CDATA[$__i > $__n]]></Calculation>"),
        setvar("$__e", "GetValue ( $__src ; $__i )"),
        setvar("$__err", "EvaluationError ( Evaluate ( $__e ) )"),
        setvar("$__r", "Evaluate ( $__e )"),
        setvar("$__out", f"$__out & $__i & Char ( 9 ) & {esc} & Char ( 9 ) & $__err & Char ( 10 )"),
        step(73, "End Loop"),
        step(7, "New Record/Request"),
        step(76, "Set Field", '<Calculation><![CDATA[$__out]]></Calculation><Field table="Tasks" id="3" name="Description"/>'),
        step(75, "Commit Records/Requests", '<NoInteract state="True"/><Option state="False"/><ESSForceCommit state="False"/>'),
    ]
    with open(sys.argv[2], "w", encoding="utf-8") as f:
        f.write('<fmxmlsnippet type="FMObjectList">' + "".join(steps) + "</fmxmlsnippet>")


if __name__ == "__main__":
    main()
