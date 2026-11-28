#!/usr/bin/env bash
# round.sh <name>: <name>.spec (written by gen_<name>.py) → <work>/<name>.fmp12,
# a copy of the Tasks template with the spec's scripts added (example
# bare_steps), checked and exported headlessly (tools/oracle/fms_check.sh);
# prints the steps of "BC Steps" as FileMaker reads them.
# Work folder: $BC_DF_DIR, default $TEMP/basecraft-datafiles.
set -uo pipefail
S=$(cd "$(dirname "$0")" && pwd)
D=${BC_DF_DIR:-${TEMP:-/tmp}/basecraft-datafiles}
mkdir -p "$D"
TASKS=${FMP12_TASKS:-"C:/Program Files/FileMaker/FileMaker Pro/DynamicTemplates/Tasks/en.fmp12"}
cd "$S/../../.."
cargo run -q -p script --example bare_steps -- "$TASKS" "$D/$1.fmp12" "$S/$1.spec" 2>/dev/null || exit 1
bash tools/oracle/fms_check.sh "$D/$1.fmp12" Admin "" "$D/$1.xml" | tail -1
python - "$D/$1.xml" <<'EOF'
import sys, re
s = open(sys.argv[1], encoding='utf-8').read()
j = s.find('<StepsForScripts'); k = s.find('name="BC Steps"', j); e = s.find('</Script>', k)
out = s[k:e]
out = re.sub(r'\s*<(UUID|OwnerID)>[^<]*</\1>', '', out)
out = re.sub(r'\s*<DDRREF[^>]*>[^<]*</DDRREF>', '', out)
out = re.sub(r'<Calculation datatype="(\d)" position="(\d+)">\s*<Calculation>\s*<Text><!\[CDATA\[(.*?)\]\]></Text>\s*</Calculation>\s*</Calculation>', r'CALC(p\2,t\1: \3)', out, flags=re.S)
out = re.sub(r'\n\s*\n', '\n', out)
out = re.sub(r'\t+', '  ', out)
sys.stdout.buffer.write(out.encode('utf-8'))
EOF
