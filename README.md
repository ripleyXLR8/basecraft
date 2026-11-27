# OpenMaker

[![CI](https://github.com/ripleyXLR8/openmaker/actions/workflows/ci.yml/badge.svg)](https://github.com/ripleyXLR8/openmaker/actions/workflows/ci.yml)

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
