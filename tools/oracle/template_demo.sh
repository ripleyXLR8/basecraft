#!/bin/sh
# Adds a table, an occurrence, relationships, lookups and value lists to a
# copy of the Tasks starter template with basecraft-cli, for Test-FmGraph
# (schema_check.ps1, with -Script template_lookup.xml):
#   template_demo.sh <Tasks en.fmp12> <out.fmp12> [basecraft-cli]
set -e
cp "$1" "$2"
F=$2
B=${3:-basecraft-cli}
s() { "$B" schema "$@" --in-place > /dev/null || { echo "failed: $*" >&2; "$B" schema "$@"; exit 1; }; }
s add-table "$F" Notes
s add-field "$F" Notes Id --type number --serial 100 --increment 10
s add-field "$F" Notes TaskKey
s add-field "$F" Notes Body --constant "first note"
s add-relationship "$F" Tasks Notes "PrimaryKey = TaskKey" --create-right --delete-right --sort-right Id:desc
s add-field "$F" Notes TaskName --lookup Tasks::Task --if-no-match value:none
s add-field "$F" Tasks NoteCount --type number --calc "Count ( Notes::Id )" --unstored
s add-field "$F" Tasks NoteList --calc "List ( Notes::Body )" --unstored
s add-occurrence "$F" Tasks "All tasks"
s add-relationship "$F" Notes "All tasks" "Id x Task"
s add-field "$F" Notes TaskTotal --type number --calc 'Count ( All tasks::PrimaryKey )' --unstored
s add-valuelist "$F" Moods Good Bad
s add-valuelist "$F" TaskNames --from-field Tasks::PrimaryKey --second Tasks::Task --second-only --sort-second
s add-field "$F" Tasks Mood --in-list Moods --max-chars 10 --message "Good or Bad"
s add-field "$F" Notes Seen --auto-calc 'TaskName & "|" & Count ( Tasks::Task ) & "|" & Count ( All tasks::Task ) & "|" & Substitute ( ValueListItems ( Get ( FileName ) ; "Moods" ) ; ¶ ; "," )'
s rename "$F" field Tasks Description Details
