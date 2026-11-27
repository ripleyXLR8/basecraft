# BaseCraft

[![CI](https://github.com/ripleyXLR8/basecraft/actions/workflows/ci.yml/badge.svg)](https://github.com/ripleyXLR8/basecraft/actions/workflows/ci.yml)
[![Progress to 1.0](https://img.shields.io/badge/progress%20to%201.0-56%25-orange)](#progress-to-10)
[![Try it in your browser](https://img.shields.io/badge/try%20it-in%20your%20browser-blue)](https://ripleyxlr8.github.io/basecraft/)

**BaseCraft is an open-source, from-scratch reimplementation, in Rust, of the
FileMaker Pro 26 desktop database application.** It aims to open, edit and save
real `.fmp12` files and to behave like the original: calculation language,
script steps, layouts, find/sort, and Browse / Find / Layout / Preview modes.

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
| Calculation engine (`crates/calc`): all 234 functions present; behaviour being checked against FileMaker | 🚧 |
| Data engine (`crates/engine`): records, find, sort, calculated fields | 🚧 |
| GUI (egui): read-only viewer, layouts drawn in form view | 🚧 |
| `.fmp12` writing: unchanged rewrite, new/edited/deleted records | 🚧 |

## Progress to 1.0

**Estimated overall progress: ~56 %** (▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░)

This is a weighted estimate over the milestones that make up version 1.0 (a
faithful single-user FileMaker Pro). Server hosting (M9) is excluded. It is
updated as milestones move; details live in [PROGRESS.md](PROGRESS.md).

| Milestone | Weight | Done |
|-----------|-------:|-----:|
| M0 Bootstrap | 2 | 100 % |
| M1 Read `.fmp12` | 15 | 82 % |
| M2 Calculation engine | 12 | 70 % |
| M3 Data engine | 12 | 65 % |
| M4 GUI (browse/find/views) | 15 | 55 % |
| M5 Script engine | 12 | 50 % |
| M6 Layout mode | 10 | 0 % |
| M7 Write `.fmp12` | 10 | 65 % |
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

* `open_file`, `describe_database`;
* `find_records`, `evaluate`;
* `create_record`, `update_record`, `delete_record`;
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
