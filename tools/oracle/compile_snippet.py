"""Builds fmxmlsnippet scripts whose steps hold calculations, so that
FileMaker compiles them (token stream key 5 and compiled form key 6).

    python compile_snippet.py exprs.txt out_prefix [size]

exprs.txt: one expression per line, or a TSV whose first column is the
expression (crates/calc/tests/oracle.tsv); `⏎` stands for a line break; `#` lines and rows whose error
column is a syntax error (12xx) are skipped. Writes out_prefix-<n>.xml with
`size` Set Variable steps each. Paste each one with Add-FmScriptSteps, close
the file, then `cargo run -p fmp12 --example calcdump -- <file>` lists the
compiled pairs (fmp12 test `compiled_forms_match_filemaker`).
"""
import sys
from xml.sax.saxutils import escape


def setvar(i, calc):
    return (f'<Step enable="True" id="141" name="Set Variable"><Value><Calculation>'
            f'<![CDATA[{calc}]]></Calculation></Value><Repetition><Calculation>'
            f'<![CDATA[1]]></Calculation></Repetition><Name>{escape(f"$v{i}")}</Name></Step>')


def main():
    size = int(sys.argv[3]) if len(sys.argv) > 3 else 400
    exprs, seen = [], set()
    for line in open(sys.argv[1], encoding="utf-8"):
        line = line.rstrip("\r\n")
        if not line.strip() or line.startswith("#"):
            continue
        cols = line.split("\t")
        if len(cols) > 2 and cols[2].startswith("12"):
            continue
        e = cols[0].replace("⏎", chr(13))
        if e not in seen and "]]>" not in e:
            seen.add(e)
            exprs.append(e)
    for n in range(0, len(exprs), size):
        steps = "".join(setvar(i, e) for i, e in enumerate(exprs[n:n + size], n))
        with open(f"{sys.argv[2]}-{n // size}.xml", "w", encoding="utf-8") as f:
            f.write('<fmxmlsnippet type="FMObjectList">' + steps + "</fmxmlsnippet>")
    print(len(exprs), "expressions")


if __name__ == "__main__":
    main()
