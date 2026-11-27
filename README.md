# OpenMaker

[![CI](https://github.com/ripleyXLR8/openmaker/actions/workflows/ci.yml/badge.svg)](https://github.com/ripleyXLR8/openmaker/actions/workflows/ci.yml)
[![Progress to 1.0](https://img.shields.io/badge/progress%20to%201.0-34%25-orange)](#progress-to-10)
[![Try it in your browser](https://img.shields.io/badge/try%20it-in%20your%20browser-blue)](https://ripleyxlr8.github.io/openmaker/)

**OpenMaker is an open-source, from-scratch reimplementation, in Rust, of the
FileMaker Pro 26 desktop database application.** It aims to open, edit and save
real `.fmp12` files and to behave like the original: calculation language,
script steps, layouts, find/sort, and Browse / Find / Layout / Preview modes.

> 🇫🇷 *OpenMaker est une recréation libre, en Rust, de FileMaker Pro 26 :
> lecture et écriture des fichiers `.fmp12`, moteur de calcul, scripts,
> layouts et interface. Avancement détaillé dans [PROGRESS.md](PROGRESS.md).*

> **Not affiliated with Claris.** FileMaker is a trademark of Claris
> International Inc. OpenMaker contains no Claris code, binaries, artwork,
> themes, templates or documentation text. File-format compatibility comes
> from reverse engineering done for interoperability.

## Status

Early development. See **[PROGRESS.md](PROGRESS.md)** for the live feature matrix.

| Area | State |
|------|-------|
| `.fmp12` reading: sectors, byte code, text, tables, fields, records | ✅ |
| `.fmp12` reading: field options, relationships, scripts, value lists, layouts, calculations | ✅ (verified against FileMaker's XML) |
| Calculation engine (`crates/calc`): 142 of 234 functions | 🚧 |
| Data engine (`crates/engine`): records, find, sort, calculated fields | 🚧 |
| GUI (egui): read-only viewer, layouts drawn in form view | 🚧 |
| `.fmp12` writing | ⏳ |

## Progress to 1.0

**Estimated overall progress: ~34 %** (▓▓▓▓▓▓▓░░░░░░░░░░░░░)

This is a weighted estimate over the milestones that make up version 1.0 (a
faithful single-user FileMaker Pro). Server hosting (M9) is excluded. It is
updated as milestones move; details live in [PROGRESS.md](PROGRESS.md).

| Milestone | Weight | Done |
|-----------|-------:|-----:|
| M0 Bootstrap | 2 | 100 % |
| M1 Read `.fmp12` | 15 | 75 % |
| M2 Calculation engine | 12 | 55 % |
| M3 Data engine | 12 | 40 % |
| M4 GUI (browse/find/views) | 15 | 30 % |
| M5 Script engine | 12 | 5 % |
| M6 Layout mode | 10 | 0 % |
| M7 Write `.fmp12` | 10 | 45 % |
| M8 Import/export, print, security… | 12 | 0 % |

## Web version

OpenMaker also runs in the browser (WebAssembly): **https://ripleyxlr8.github.io/openmaker/**.
Open or drop a `.fmp12` file. It stays on your machine, because everything runs
locally in the page. To build it yourself: `cd crates/app && trunk serve`.

## Layout of the repository

```
crates/fmp12   .fmp12 (HBAM7) reader, later writer
crates/app     the OpenMaker desktop application (egui)
tools/fmdump   command-line inspector for .fmp12 files
tools/oracle   scripts that drive a real FileMaker Pro to produce test oracles
docs/          format specification and reference lists
```

Also: `crates/calc` (calculation language) and `crates/engine` (records,
found sets, find, sort). Planned: `script` (script engine).

## Try it

```sh
cargo run -p openmaker -- MyDatabase.fmp12   # GUI (set OPENMAKER_LANG=fr for French)
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
