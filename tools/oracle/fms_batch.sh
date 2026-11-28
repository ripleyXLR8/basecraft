#!/usr/bin/env bash
# Consistency-checks many .fmp12 files in one SSH round trip (see
# fms_check.sh for one file and its XML export).
#
#   tools/oracle/fms_batch.sh a.fmp12 b.fmp12 …
#
# Prints one line per file: `OK <file>` or `BAD <file>: <error lines>`;
# exit code 0 when every file passes. Same environment as fms_check.sh.
set -euo pipefail
[ $# -gt 0 ] || { echo "usage: fms_batch.sh file.fmp12 …" >&2; exit 2; }
host=${FMS_SSH:-basecraft@127.0.0.1}
port=${FMS_PORT:-2222}
key=${FMS_KEY:-$HOME/.ssh/basecraft_vm}
opts=(-i "$key" -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
tool='/opt/FileMaker/FileMaker Server/Database Server/bin/FMDeveloperTool'
dir=$(ssh "${opts[@]}" -p "$port" "$host" 'mktemp -d')
trap 'ssh "${opts[@]}" -p "$port" "$host" "rm -rf $dir"' EXIT
# Number the copies: names may repeat (en.fmp12 in every template folder).
tmp=$(mktemp -d)
i=0
for f in "$@"; do
  cp "$f" "$tmp/$(printf '%04d' $i).fmp12"
  i=$((i + 1))
done
tar -C "$tmp" -cf - . | ssh "${opts[@]}" -p "$port" "$host" "tar -C $dir -xf -"
rm -rf "$tmp"
out=$(ssh "${opts[@]}" -p "$port" "$host" "cd $dir && for f in *.fmp12; do r=\$('$tool' --checkConsistency \$f 2>&1); if grep -q 'found no problems' <<<\"\$r\"; then echo \"\$f OK\"; else echo \"\$f BAD \$(grep -iE 'error|bad block' <<<\"\$r\" | tr '\\n' ' ' | cut -c1-300)\"; fi; done")
status=0
i=0
for f in "$@"; do
  line=$(grep "^$(printf '%04d' $i).fmp12 " <<<"$out" || echo "x MISSING")
  case $line in
    *" OK") echo "OK $f" ;;
    *) echo "BAD $f: ${line#* BAD }"; status=1 ;;
  esac
  i=$((i + 1))
done
exit $status
