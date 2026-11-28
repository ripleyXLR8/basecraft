#!/usr/bin/env bash
# Checks .fmp12 files with FileMaker Server's FMDeveloperTool, headless, in
# the local test VM (see tools/oracle/README.md, "FileMaker Server VM").
#
#   tools/oracle/fms_check.sh file.fmp12 [account] [password] [out.xml]
#
# Prints FMDeveloperTool's consistency report; exit code 0 when it finds no
# problems. With an output path, also writes FileMaker's Save as XML export
# there (account "Admin" and an empty password by default: test files only).
#
# Environment: FMS_SSH (default "basecraft@127.0.0.1"), FMS_PORT (2222),
# FMS_KEY (~/.ssh/basecraft_vm).
set -euo pipefail
f=${1:?usage: fms_check.sh file.fmp12 [account] [password] [out.xml]}
acct=${2:-Admin}
pass=${3:-}
out=${4:-}
host=${FMS_SSH:-basecraft@127.0.0.1}
port=${FMS_PORT:-2222}
key=${FMS_KEY:-$HOME/.ssh/basecraft_vm}
opts=(-i "$key" -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
tool='/opt/FileMaker/FileMaker Server/Database Server/bin/FMDeveloperTool'
dir=$(ssh "${opts[@]}" -p "$port" "$host" 'mktemp -d')
trap 'ssh "${opts[@]}" -p "$port" "$host" "rm -rf $dir"' EXIT
scp -q "${opts[@]}" -P "$port" "$f" "$host:$dir/check.fmp12"
status=0
report=$(ssh "${opts[@]}" -p "$port" "$host" "cd $dir && '$tool' --checkConsistency check.fmp12" 2>&1) || status=$?
echo "$report"
grep -q "found no problems" <<<"$report" || status=1
if [ -n "$out" ]; then
  ssh "${opts[@]}" -p "$port" "$host" "cd $dir && '$tool' --saveAsXML check.fmp12 '$acct' '$pass' -t $dir/out.xml -f >/dev/null"
  scp -q "${opts[@]}" -P "$port" "$host:$dir/out.xml" "$out"
fi
exit $status
