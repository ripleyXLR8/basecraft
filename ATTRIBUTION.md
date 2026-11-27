# Attribution

| Path | What | Author / source | Licence | How it was made |
|------|------|-----------------|---------|-----------------|
| `assets/icon/*` | BaseCraft app icon (purple tile, stack of record cards) | BaseCraft contributors | MIT (this repository) | Original artwork drawn in code by `packaging/icon.py` (Pillow), in the general style of the Craft apps (flat tile, outlined cream object); nothing traced or copied |
| `crates/fmp12/assets/blank.fmp12` | Empty database used by `fmp12::schema_edit::new_file` (one table "Blank", its occurrence and an empty layout) | BaseCraft contributors | MIT (this repository) | Created with FileMaker Pro 26 (French trial): File › Create › Blank, opened and closed once; then `cargo run -p fmp12 --example mkblank` removed the five default fields, replaced FileMaker's theme with BaseCraft's own plain theme (styles written in code) and the OS user name in object metadata with "BaseCraft". What remains is the structure FileMaker writes in every new file (catalog headers, the Admin account, the standard privilege sets and custom-menu names, file formats in the French locale) |
| `crates/fmp12/tests/fixtures/charts.fmp12` | Test file with 14 charts, one per chart type and data source | BaseCraft contributors | MIT (this repository) | Built with `basecraft-cli` (new file, tables and records), then charts were added in FileMaker Pro 26 (French trial) on its layout; contains no template content |
| `crates/fmp12/src/collate.rs` weights, `crates/fmp12/tests/fixtures/compiled.tsv` | Collation weights of names, compiled calculations | BaseCraft contributors | MIT (this repository) | Measured by giving FileMaker Pro 26 our own field names and calculation expressions (`tools/oracle/compile_snippet.py`, `compile_exprs.txt`, `crates/calc/tests/oracle.tsv`) and reading the bytes it wrote |
| `crates/fmp12` sector and chunk layer | Format notes | [fmptools](https://github.com/evanmiller/fmptools) by Evan Miller | MIT | Public notes and source used as a starting point; see `docs/FORMAT.md` |

BaseCraft contains no Claris code, binaries, artwork, themes, templates or
documentation text. Test fixtures derived from FileMaker starter templates
are never committed; the tests that use them are skipped when they are absent.
