#!/usr/bin/env bash
# Builds BCRemote.fmp12, the Data API test file hosted on the FileMaker
# Server VM (tools/oracle/README.md), with basecraft-cli only:
#   table Contacts (Name, City, Age, Photo), three records, all fields on
#   the Contacts layout, script Echo, privilege set "API Access" (a copy of
#   [Data Entry Only] with fmrest and fmodata), accounts api and Admin.
#
#   tools/oracle/remote_fixture.sh out.fmp12 [basecraft-cli]
#
# The password of both accounts is read from FMS_TEST_PASSWORD (default
# "test123": a throwaway password for this local test file only).
set -euo pipefail
out=${1:?usage: remote_fixture.sh out.fmp12 [basecraft-cli]}
cli=${2:-target/debug/basecraft-cli}
pw=${FMS_TEST_PASSWORD:-test123}
tmp="$out.tmp.fmp12"
rm -f "$tmp" "$out"
q() { "$cli" "$@" >/dev/null; }
q new-file "$tmp" --table Contacts
q schema add-field "$tmp" Contacts Name --type text --in-place
q schema add-field "$tmp" Contacts City --type text --in-place
q schema add-field "$tmp" Contacts Age --type number --in-place
q schema add-field "$tmp" Contacts Photo --type container --in-place
q new "$tmp" Contacts "Name=Ada" "City=London" "Age=36" --in-place
q new "$tmp" Contacts "Name=Grace" "City=Arlington" "Age=85" --in-place
q new "$tmp" Contacts "Name=Linus" "City=Helsinki" "Age=29" --in-place
cargo run -q -p remote --example fixture -- "$tmp" "$out"
rm -f "$tmp"
# The blank file's sets are named in French ([Saisie de données uniquement]).
dataentry=$("$cli" privileges "$out" | grep -o '"name": "\[[^"]*\]"' | sed -n 2p | cut -d'"' -f4)
q privileges add "$out" "API Access" --copy-of "$dataentry" --extended fmrest,fmodata --in-place
q accounts add "$out" api --new-password "$pw" --set "API Access" --in-place
q accounts set "$out" Admin --new-password "$pw" --in-place --account Admin --password ""
echo "$out"
