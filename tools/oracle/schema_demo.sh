#!/bin/sh
# Builds a file with basecraft-cli covering every schema edit, for
# Test-FmSchema (schema_check.ps1): schema_demo.sh <out.fmp12> [basecraft-cli]
set -e
F=$1
B=${2:-basecraft-cli}
s() { "$B" schema "$@" --in-place > /dev/null; }
"$B" new-file "$F" --table Contacts > /dev/null
s add-field "$F" Contacts Name
s add-table "$F" People
s add-field "$F" People Id --type number --serial 1 --unique --prohibit-modification
s add-field "$F" People Name --not-empty --comment "Full name"
s add-field "$F" People Age --type number --range 0:150 --validate "Age >= 0"
s add-field "$F" People Created --type timestamp --stamp creation-timestamp
s add-field "$F" People Modified --type timestamp --stamp modification-timestamp
s add-field "$F" People Country --constant France --index all
s add-field "$F" People Key --auto-calc "Get ( UUID )"
s add-field "$F" People Code --auto-calc 'Upper ( Left ( Country ; 2 ) ) & "-" & Id'
s add-field "$F" People Note --global
s add-field "$F" People Phones --repetitions 3
s add-field "$F" People Photo --type container
s add-field "$F" People Born --type date --existing
s add-field "$F" People Alarm --type time
s add-field "$F" People Label --calc 'Country & " #" & Id & " " & Case ( Id > 1 ; "many" ; "first" )'
s add-field "$F" People Double --type number --calc "Id * 2" --unstored
s add-field "$F" People Total --summary total:Id
s add-field "$F" People Count --summary count:Id
s add-field "$F" People Tmp --type date
s set-options "$F" People Name --no-not-empty
s rename "$F" field People Note "Global note"
s delete "$F" field People Tmp
s add-table "$F" Scratch
s rename "$F" table Scratch Scratch2
s delete "$F" table Scratch2
