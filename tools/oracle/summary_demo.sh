#!/bin/sh
# Builds a file for checking summary options in FileMaker's Table view:
# summary_demo.sh <out.fmp12> <basecraft-cli> <base.fmp12>
#
# A Sales table (Region, Amount, Weight, 7 records) is added to a copy of
# <base.fmp12> (a template: its edit boxes give the new field objects their
# data formats), with running totals and counts (restarting by Region or
# not), a weighted average, fractions of total (subtotaled by Region or
# not), standard deviations (sample, by population) and GetSummary
# calculations by Region. Then put the fields on the Sales layout:
#   cargo run -p fmp12 --example fieldsonlayout -- <out.fmp12> Sales <shown.fmp12>
# open a copy of that file, choose the Sales layout, Table view, sort by
# Region (column header menu, Sort ascending) and read the values.
# BaseCraft's expectations: crates/engine/src/summary.rs
# (summary_options_match_filemaker).
set -e
F=$1
B=$2
rm -f "$F"
s() { "$B" schema "$@" --in-place > /dev/null || { echo "failed: $*" >&2; "$B" schema "$@"; exit 1; }; }
cp "$3" "$F"
s add-table "$F" Sales
s add-field "$F" Sales Region
s add-field "$F" Sales Amount --type number
s add-field "$F" Sales Weight --type number
s add-field "$F" Sales RunTot --summary total:Amount --running
s add-field "$F" Sales RunTotR --summary total:Amount --running --restart-by Region
s add-field "$F" Sales RunCnt --summary count:Amount --running
s add-field "$F" Sales RunCntR --summary count:Amount --running --restart-by Region
s add-field "$F" Sales WAvg --summary average:Amount --weighted-by Weight
s add-field "$F" Sales Frac --summary fraction:Amount
s add-field "$F" Sales FracSub --summary fraction:Amount --subtotal-by Region
s add-field "$F" Sales SD --summary stdev:Amount
s add-field "$F" Sales SDP --summary stdev:Amount --by-population
s add-field "$F" Sales gRunTot --type number --calc "GetSummary ( RunTot ; Region )" --unstored
s add-field "$F" Sales gRunTotR --type number --calc "GetSummary ( RunTotR ; Region )" --unstored
s add-field "$F" Sales gRunCntR --type number --calc "GetSummary ( RunCntR ; Region )" --unstored
s add-field "$F" Sales gWAvg --type number --calc "GetSummary ( WAvg ; Region )" --unstored
s add-field "$F" Sales gFrac --type number --calc "GetSummary ( Frac ; Region )" --unstored
s add-field "$F" Sales gFracSub --type number --calc "GetSummary ( FracSub ; Region )" --unstored
s add-field "$F" Sales gSDP --type number --calc "GetSummary ( SDP ; Region )" --unstored
n() { "$B" new "$F" Sales "$@" --in-place > /dev/null; }
n Region=A Amount=10 Weight=1
n Region=B Amount=5 Weight=4
n Region=A Amount=30 Weight=2
n Region=C Amount=40 Weight=2
n Region=B Amount=15 Weight=1
n Region=C Weight=1
n Region=A Amount=20 Weight=3
