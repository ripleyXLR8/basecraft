<p align="center">
  <img src="assets/icon/basecraft-256.png" width="128" height="128" alt="BaseCraft icon: a stack of record cards on a purple tile">
</p>

<h1 align="center">BaseCraft</h1>

<p align="center">
  <b>Databases you design, browse and script; an open-source desktop database built from scratch in pure Rust.</b>
</p>

<p align="center">
  Tables, relationships, layouts, calculations, scripts, find and sort, in a familiar
  workflow with Browse, Find, Layout and Preview modes. It opens and saves FileMaker
  <code>.fmp12</code> files, runs natively on Windows, macOS and Linux, and in the browser
  through WebAssembly.
</p>

<p align="center">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-7040aa?style=flat-square&logo=rust&logoColor=white">
  <img alt="Runs on Windows, macOS, Linux and the web" src="https://img.shields.io/badge/runs%20on-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux%20%C2%B7%20Web-8a5cc2?style=flat-square">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-7040aa?style=flat-square">
  <img alt="Agent-drivable over MCP and CLI" src="https://img.shields.io/badge/agents-MCP%20%C2%B7%20CLI-8a5cc2?style=flat-square">
  <br>
  <a href="https://github.com/ripleyXLR8/basecraft/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ripleyXLR8/basecraft/actions/workflows/ci.yml/badge.svg"></a>
  <a href="#progress-to-10"><img alt="Progress to 1.0" src="https://img.shields.io/badge/progress%20to%201.0-77%25-orange?style=flat-square"></a>
</p>

<p align="center">
  <a href="https://ripleyxlr8.github.io/basecraft/"><b>Try BaseCraft in your browser</b></a> ·
  <a href="PROGRESS.md">Progress</a> ·
  <a href="docs/FORMAT.md">The .fmp12 format</a> ·
  <a href="#command-line-for-scripts-and-ai-agents">CLI and MCP</a>
</p>

<p align="center">
  <a href="#status">Status</a> ·
  <a href="#progress-to-10">Progress to 1.0</a> ·
  <a href="#command-line-for-scripts-and-ai-agents">Command line</a> ·
  <a href="#web-version">Web version</a> ·
  <a href="#try-it">Try it</a> ·
  <a href="#testing">Testing</a>
</p>

> 🇫🇷 *BaseCraft est une recréation libre, en Rust, de FileMaker Pro 26 :
> lecture et écriture des fichiers `.fmp12`, moteur de calcul, scripts,
> layouts et interface. Avancement détaillé dans [PROGRESS.md](PROGRESS.md).*

> **Not affiliated with Claris.** FileMaker is a trademark of Claris
> International Inc. BaseCraft contains no Claris code, binaries, artwork,
> themes, templates or documentation text. File-format compatibility comes
> from reverse engineering done for interoperability.

## Status

Early development. See **[PROGRESS.md](PROGRESS.md)** for the live feature matrix.

| Area | State |
|------|-------|
| `.fmp12` reading: sectors, byte code, text, tables, fields, records | ✅ |
| `.fmp12` reading: field options, relationships, scripts, value lists, layouts, calculations | ✅ (verified against FileMaker's XML) |
| Calculation engine (`crates/calc`): all 234 functions; 1,934 of 1,987 oracle expressions match FileMaker Pro 26 | 🚧 |
| Data engine (`crates/engine`): records, find, sort, calculated fields | 🚧 |
| GUI (egui): read-only viewer, layouts drawn in form view | 🚧 |
| `.fmp12` writing: unchanged rewrite, new/edited/deleted records | 🚧 |

## Progress to 1.0

**Estimated overall progress: ~77 %** (▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░)

This is a weighted estimate over the milestones that make up version 1.0 (a
faithful single-user FileMaker Pro). Server hosting (M9) is excluded. It is
updated as milestones move; details live in [PROGRESS.md](PROGRESS.md).

| Milestone | Weight | Done |
|-----------|-------:|-----:|
| M0 Bootstrap | 2 | 100 % |
| M1 Read `.fmp12` | 15 | 82 % |
| M2 Calculation engine | 12 | 90 % |
| M3 Data engine | 12 | 85 % |
| M4 GUI (browse/find/views) | 15 | 55 % |
| M5 Script engine | 12 | 80 % |
| M6 Layout mode | 10 | 50 % |
| M7 Write `.fmp12` | 10 | 85 % |
| M8 Import/export, print, security… | 12 | 45 % |

## Command line for scripts and AI agents

`basecraft-cli` drives BaseCraft without a GUI. Every command prints **JSON**,
so tools and AI agents can read the results directly. Nothing is written to disk unless
you pass `-o <out.fmp12>` or `--in-place`; without them, writes are dry runs.

```sh
basecraft-cli info    Contacts.fmp12                       # tables, fields, relationships, layouts, value lists
basecraft-cli records Contacts.fmp12 Contacts --find "First Name=ad" --sort "Last Name:desc" --limit 20
basecraft-cli eval    Contacts.fmp12 'Upper ( ${Last Name} )' --table Contacts --record 2
basecraft-cli new     Contacts.fmp12 Contacts "First Name=Ada" "Last Name=Lovelace" -o out.fmp12
basecraft-cli set     Contacts.fmp12 Contacts 2 "Company=Navy" --in-place
basecraft-cli delete  Contacts.fmp12 Contacts 2 --in-place
basecraft-cli scripts Contacts.fmp12                       # scripts and their steps
basecraft-cli run-script Contacts.fmp12 "Sort by Name" --layout "Contact List" [--param P] [--answer 2] [-o out.fmp12]
```

* `records` uses FileMaker's find syntax (`=`, `==`, `*`, `<`, `>`, ranges `a...b`,
  `--omit` requests) and FileMaker's sort.
* `new` applies auto-enter options (serials, UUIDs, timestamps). `new` and `set` report
  validation failures in `validation_errors`.
* In `eval`, field names that contain spaces go between `${…}`.
* **Session mode** keeps a file open and reads one JSON command per line on stdin, replying
  with one JSON line each:

```sh
basecraft-cli session Contacts.fmp12
{"cmd":"records","args":["Contacts","--find","Company=Navy"]}
{"cmd":"set","args":["Contacts","1","Company=Army"]}
{"cmd":"save","args":["-o","out.fmp12"]}
{"cmd":"quit"}
```

Files written by the CLI open in FileMaker Pro 26.

### Manage Database: new files, tables, fields, relationships and value lists

```sh
basecraft-cli new-file   Shop.fmp12 --table Customers               # a new, empty database
basecraft-cli schema add-table   Shop.fmp12 Orders --in-place         # table + occurrence + layout
basecraft-cli schema add-field   Shop.fmp12 Orders Id --type number --serial 1 --unique --in-place
basecraft-cli schema add-field   Shop.fmp12 Orders Amount --type number --range 0:10000 --in-place
basecraft-cli schema add-field   Shop.fmp12 Orders Label --calc '"#" & Id & " " & Amount' --in-place
basecraft-cli schema add-field   Shop.fmp12 Orders Total --summary total:Amount --in-place
basecraft-cli schema set-options Shop.fmp12 Orders Amount --not-empty --index all --in-place
basecraft-cli schema rename      Shop.fmp12 field Orders Amount Price --in-place
basecraft-cli schema rename      Shop.fmp12 table Orders Sales --in-place
basecraft-cli schema delete      Shop.fmp12 field Sales Label --in-place
basecraft-cli schema delete      Shop.fmp12 table Sales --in-place
```

```sh
# Customers (Id, Name, Limit) and Orders (CustomerId, Amount, Date):
basecraft-cli schema add-occurrence   Shop.fmp12 Orders "Big orders" --in-place      # another occurrence of Orders
basecraft-cli schema add-relationship Shop.fmp12 Customers Orders "Id = CustomerId" \
    --create-right --delete-right --sort-right Date:desc --in-place
basecraft-cli schema add-relationship Shop.fmp12 Customers "Big orders" "Id = CustomerId" "Limit < Amount" --in-place
basecraft-cli schema set-relationship Shop.fmp12 Customers Orders --no-delete-right --in-place
basecraft-cli schema add-field        Shop.fmp12 Orders CustomerName --lookup Customers::Name --if-no-match value:? --in-place
basecraft-cli schema add-valuelist    Shop.fmp12 Sizes S M L --in-place
basecraft-cli schema add-valuelist    Shop.fmp12 Customers --from-field Customers::Id --second Customers::Name --second-only --in-place
basecraft-cli schema add-field        Shop.fmp12 Orders Size --in-list Sizes --max-chars 1 --message "S, M or L" --in-place
basecraft-cli schema rename           Shop.fmp12 occurrence "Big orders" Large --in-place
basecraft-cli schema delete           Shop.fmp12 relationship Customers Large --in-place   # also: occurrence, valuelist
```

* Field options: `--type text|number|date|time|timestamp|container`, `--comment`;
  `--calc FORMULA` (`--unstored`) or `--summary total|average|count|minimum|maximum|stdev|fraction|list:Field`;
  auto-enter `--serial NEXT`, `--stamp creation-date|creation-time|creation-timestamp|creation-name|creation-account`
  (or `modification-…`), `--auto-calc FORMULA` (`--replace`, `--always-evaluate`), `--constant VALUE`,
  `--prohibit-modification`; validation `--not-empty`, `--unique`, `--existing`, `--range LOW:HIGH`,
  `--validate FORMULA`, `--always-validate`, `--strict` (no override), `--in-list VALUELIST`,
  `--max-chars N`, `--message TEXT` (or `--message-calc FORMULA`); serial `--increment N`;
  lookups `--lookup Occurrence::Field` (`--lookup-from Occurrence`, `--if-no-match
  none|lower|higher|value:TEXT`, `--copy-empty`); summaries `--running`, `--restart-by Field`,
  `--weighted-by Field`, `--subtotal-by Field`; storage `--global`, `--repetitions N`,
  `--index none|minimal|all`, `--no-auto-index`. With `set-options`, `--name NEW` renames and
  `--no-<option>` turns an option off.
* Relationships join two table occurrences with one or more `Field op Field` arguments, the
  operator being `=`, `≠` (`!=`), `<`, `≤` (`<=`), `>`, `≥` (`>=`) or `×` (`x`). Options per
  side: `--create-left/right`, `--delete-left/right`, `--sort-left/right Field[:desc]`
  (repeated); `set-relationship` takes the same arguments (new predicates replace the old
  ones) and `--no-…`. As in FileMaker, two occurrences have at most one relationship and the
  graph has no cycles: relate a new occurrence instead.
* Value lists take custom values as arguments, or `--from-field Occurrence::Field` with
  `--second Occurrence::Field`, `--second-only`, `--sort-second`, `--related-from Occurrence`.
* Lookups copy when the match field changes (`set`, `new`, scripts) as in FileMaker, and
  deleting a record deletes the related records its relationships say to delete.
* Calculations use FileMaker's syntax (fields of other tables as `Occurrence::Field`) and are
  compiled the way FileMaker stores them, so FileMaker evaluates them.
* A new table gets a table occurrence and an empty layout of the same name. Deleting a table
  also deletes its occurrences and their relationships.

### Import, export and PDF

```sh
basecraft-cli export    Contacts.fmp12 Contacts -o out.csv --field "First Name" --field "Phone Numbers::Number" --sort "Last Name"
basecraft-cli import    Contacts.fmp12 Contacts people.xlsx --names --mode update --match Email --add-remaining -o out.fmp12
basecraft-cli print-pdf Contacts.fmp12 "Contact List" -o list.pdf --paper a4 --landscape --sort Initial
```

* `export` writes the found set (`--find`, `--omit`, `--sort`) as `csv`, `tab`, `mer`
  (Merge), `xml` (FMPXMLRESULT), `xlsx` or `json`, chosen by `--format` or the `-o`
  extension. Without `-o`, text formats are returned in `content`. `--field` picks
  fields (`Occurrence::Field` for related fields; default: every field). `--formatted`
  exports values in the file's display formats instead of as stored. As in FileMaker,
  returns inside fields become vertical tabs, and repetitions and related records are
  joined with the group separator (ASCII 29).
* `import` reads `csv`, `tab`, `mer` or `xlsx`. `--names` takes field names from the
  first row (columns map by name); `--map Field` (repeated) maps columns in order.
  `--mode add` (default), `update` (records of the found set whose `--match` fields
  equal the row's; `--add-remaining` adds the others) or `replace` (row *n* into
  record *n* of the found set). Auto-enter applies unless `--no-auto-enter`;
  validation failures are reported, and records failing a field that validates
  "always" are skipped. Like `new`, it writes only with `-o` or `--in-place`.
* `print-pdf` lays the layout out as Preview mode does (header and footer on every
  page, body per record, sub-summaries for the sorted break fields, grand summaries,
  summary fields) on `--paper letter|a4`, `--landscape`, `--margin` points.

### MCP server (Claude and other AI agents)

`basecraft-cli mcp [file.fmp12]` is a [Model Context Protocol](https://modelcontextprotocol.io)
server on stdio. It exposes these tools:

* `open_file`, `new_file`, `describe_database`;
* `find_records`, `evaluate`;
* `create_record`, `update_record`, `delete_record`;
* `create_table`, `create_field` (types, calculations, summaries, auto-enter, lookups, validation, storage);
* `create_occurrence`, `create_relationship`, `create_valuelist`;
* `list_scripts`, `run_script`;
* `export_records`, `import_records`, `print_pdf`;
* `save_file`.

Changes stay in memory until `save_file` is called.

```sh
cargo install --path crates/cli                      # installs basecraft-cli
claude mcp add basecraft -- basecraft-cli mcp         # Claude Code
```

Claude Desktop (`claude_desktop_config.json`):

```json
{ "mcpServers": { "basecraft": { "command": "basecraft-cli", "args": ["mcp"] } } }
```

## Web version

BaseCraft also runs in the browser (WebAssembly): **https://ripleyxlr8.github.io/basecraft/**.
Open or drop a `.fmp12` file. It stays on your machine, because everything runs
locally in the page. To build it yourself: `cd crates/app && trunk serve`.

## Layout of the repository

```
crates/fmp12   .fmp12 (HBAM7) reader, later writer
crates/app     the BaseCraft desktop application (egui)
crates/cli     basecraft-cli: JSON command line for scripts and AI agents
crates/print   Preview-mode pagination and PDF output
tools/fmdump   command-line inspector for .fmp12 files
tools/oracle   scripts that drive a real FileMaker Pro to produce test oracles
docs/          format specification and reference lists
```

Also: `crates/calc` (calculation language) and `crates/engine` (records,
found sets, find, sort). Planned: `script` (script engine).

## Try it

```sh
cargo run -p basecraft -- MyDatabase.fmp12   # GUI (set BASECRAFT_LANG=fr for French)
```

```sh
cargo run -p fmdump -- MyDatabase.fmp12 tables
cargo run -p fmdump -- MyDatabase.fmp12 records "Contacts"
cargo run -p fmdump -- MyDatabase.fmp12 chunks     # raw path/key/value dump
```

## Testing

```sh
cargo test --workspace
```

If FileMaker Pro is installed, some tests also run against its starter
templates. Point `FMP12_TEMPLATES` at a template folder to use another
location. Without them, those tests are skipped, as on CI.

## Documentation

* [docs/FORMAT.md](docs/FORMAT.md): our specification of the `.fmp12` format.
* [docs/reference/](docs/reference): lists of calculation and `Get()` functions, used to measure coverage.

## Credits

The sector and byte-code layer is based on the public notes of
[fmptools](https://github.com/evanmiller/fmptools) (MIT) by Evan Miller.

## License

MIT, see [LICENSE](LICENSE).
