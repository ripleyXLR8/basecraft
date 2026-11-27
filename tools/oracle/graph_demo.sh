#!/bin/sh
# Builds a file with basecraft-cli covering occurrences, relationships,
# lookups, value lists and the other field options, for Test-FmGraph
# (schema_check.ps1): graph_demo.sh <out.fmp12> [basecraft-cli]
#
# Clients::Id and Invoices::ClientId get the constant 1, so records created
# in FileMaker on the new (empty) layouts relate to each other; the
# auto-enter calculations named Seen store what FileMaker sees through the
# relationship and the value lists when it creates the record.
set -e
F=$1
B=${2:-basecraft-cli}
s() { "$B" schema "$@" --in-place > /dev/null || { echo "failed: $*" >&2; "$B" schema "$@"; exit 1; }; }
"$B" new-file "$F" --table Clients > /dev/null
s add-field "$F" Clients Id --type number --constant 1
s add-field "$F" Clients Name --constant Ada
s add-field "$F" Clients City --constant Paris
s add-table "$F" Invoices
s add-field "$F" Invoices Id --type number --serial 1 --increment 5
s add-field "$F" Invoices ClientId --type number --constant 1
s add-field "$F" Invoices Amount --type number --constant 10
s add-relationship "$F" Clients Invoices "Id = ClientId" --create-right --delete-right --sort-right Amount:desc --sort-left Name
s add-field "$F" Invoices ClientName --lookup Clients::Name --if-no-match value:unknown
s add-field "$F" Invoices ClientCity --lookup Clients::City --if-no-match lower --copy-empty
s add-valuelist "$F" Cities Paris Lyon Nice
s add-valuelist "$F" ClientNames --from-field Clients::Id --second Clients::Name --second-only --sort-second
s add-valuelist "$F" Amounts --from-field Invoices::Amount --related-from Clients
s add-field "$F" Clients Town --in-list Cities --max-chars 20 --message "Pick a city"
s add-field "$F" Clients InvoiceCount --type number --calc "Count ( Invoices::Id )" --unstored
s add-field "$F" Clients Amounts --calc "List ( Invoices::Amount )" --unstored
s add-field "$F" Clients Seen --auto-calc 'Count ( Invoices::Id ) & "|" & List ( Invoices::Amount ) & "|" & Substitute ( ValueListItems ( Get ( FileName ) ; "Cities" ) ; ¶ ; "," ) & "|" & ValueListItems ( Get ( FileName ) ; "ClientNames" )'
s add-field "$F" Invoices Seen --auto-calc 'Clients::Name & "|" & Count ( Clients::Id )'
s add-field "$F" Invoices Running --summary total:Amount --running --restart-by ClientId
s add-field "$F" Invoices Weighted --summary average:Amount --weighted-by Id
s add-field "$F" Invoices Share --summary fraction:Amount --subtotal-by ClientId
s add-field "$F" Invoices Rows --summary count:Id --running
# Occurrences named differently from their table, every join operator.
s add-occurrence "$F" Invoices "Big invoices"
s add-relationship "$F" Clients "Big invoices" "Id = ClientId" "Id < Amount" --delete-left
s add-occurrence "$F" Clients "Other clients"
s add-relationship "$F" Invoices "Other clients" "ClientId ≠ Id" "Amount ≥ Id" "Amount > Id" "Id <= Id" "Id x Name"
s add-field "$F" Clients Big --type number --calc 'Count ( Big invoices::Id )' --unstored
# Edits: an occurrence renamed and deleted, a relationship changed and
# deleted, a value list renamed and deleted.
s add-occurrence "$F" Clients Tmp
s rename "$F" occurrence Tmp Tmp2
s add-relationship "$F" Tmp2 "Big invoices" "Id = Id"
s set-relationship "$F" Tmp2 "Big invoices" "Id = Amount" --create-left
s delete "$F" relationship Tmp2 "Big invoices"
s delete "$F" occurrence Tmp2
s add-valuelist "$F" Scratch a b
s rename "$F" valuelist Scratch Scratch2
s delete "$F" valuelist Scratch2
