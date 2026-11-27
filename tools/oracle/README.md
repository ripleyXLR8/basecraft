# Oracle tooling

Scripts that drive a locally installed, licensed (or trial) FileMaker Pro 26
on Windows to produce **ground truth** for BaseCraft's tests. Nothing here
ships Claris content. The outputs stay on your machine unless they were built
from files we authored ourselves.

* `fmui.ps1`: UI Automation helpers. Dot-source it in PowerShell.
  * `Start-Fm <file>` / `Stop-Fm`: launch FileMaker on a file and dismiss the trial dialog.
  * `Export-FmXml <out.xml>`: *Tools › Save a Copy as XML*, with DDR details and per-object binary data.
  * `Save-FmScreen <name>`: screenshot of the FileMaker window only.
  * `Click-Fm x y`, `Send-FmKeys`, `Show-FmTree`: low-level driving.
  Coordinates assume the French UI at the default window size.
  Prerequisite: *Preferences › Use advanced tools* enabled.
* `uuidmap.py <name>`: matches object UUIDs in `<name>.xml` (a Save-as-XML
  export) with 16-byte values in `<name>.raw` (`fmdump <file> raw`), which
  maps each XML catalog to its binary path.
* `Test-FmOpen <file>` (in `fmui.ps1`): opens a file and reports `OPEN` or the text of the blocking dialog.
* `Add-FmScriptSteps <snippet.xml>` (in `fmui.ps1`): pastes the steps of an
  `fmxmlsnippet` into a new script of the open file through the clipboard
  (`Set-FmClip`, format `Mac-XMSS` = u32 LE length + UTF-8 XML); `Close-Fm`
  then quits and waits for the file to be complete.
  `step_params.xml` is the 57-step script used to decode step parameters
  (field and layout ids of the Tasks template).
* `step_ids.py`: reverse oracle for the step catalogue. `mutate` rewrites
  every step id of a copy with consecutive ids, `read` lists `id name` from
  the XML exports (`docs/reference/script_steps.txt`).
* `mutate.py`, `xmldiff.py`, `attrib_fields.py`: reverse oracle. Flip bits in copies, export XML, attribute the differences.
* `token_sweep_analyze.py`: reads the XML of files whose calculation token bytes were overwritten (one way calc_tables.rs was built).
* Index and leaf experiments are Rust examples of `fmp12`:
  `cargo run -p fmp12 --example index -- <file>` (separators next to each child's first entry),
  `swaproot <in> <out>` (rebuild only the root index), `releaf <in> [out]` (re-pack every leaf in place).
* Saving records: `cargo run -p engine --example edit -- <in> <out> new:Products "set:Products:+:Name=x" del:Products:3 …`
  writes a copy through `Database::save`; open it with `Test-FmOpen` and read
  fields back through UI Automation (`Edit` controls named `Field: Table::Field`).
* Manage Database helpers (in `fmui.ps1`): `Open-FmManageDb`, `Select-FmTab`,
  `Get-FmDbRows` (rows of the Tables or Fields list as FileMaker shows them),
  `Add-FmFields <snippet.xml>` (pastes fields, clipboard format `Mac-XMFD`),
  `Save-FmWindowShot <element> <name>` (screenshot of one FileMaker dialog).
* `schema_check.ps1`: `Test-FmSchema <file> <Table> [-Records n] [-Layout i] [-Xml out.xml]`
  checks a file whose schema BaseCraft wrote: opens a copy, reports the
  damage dialog if any, lists the Tables and Fields rows of Manage Database
  (screenshots too), creates records on the table's layout and optionally
  saves the copy as XML. `schema_demo.sh <out.fmp12> [basecraft-cli]` builds
  the file used for the check with `basecraft-cli new-file` and `schema`.
* `Test-FmGraph <file> [-Plan "1:1,2:2,1:1"] [-Script steps.xml] [-Xml out.xml]` (in
  `schema_check.ps1`) checks relationships, lookups and value lists BaseCraft wrote: opens a
  copy, screenshots Manage Database's Relationships tab, lists Manage Value Lists, creates
  records on the layouts of `-Plan`, pastes and runs the steps of `-Script` in a new script,
  optionally saves the copy as XML. `graph_demo.sh <out.fmp12> [basecraft-cli]` builds the
  file it was written for (run it with `-Script graph_lookup.xml`), `template_demo.sh <Tasks
  en.fmp12> <out.fmp12>` does the same on a copy of the Tasks template (`-Plan ""
  -Script template_lookup.xml`). The auto-enter calculations named `Seen` store what
  FileMaker saw through the relationships when it created each record.
* More helpers in `fmui.ps1`: `Find-FmWindow <pattern>`, `Click-FmId <automation id> [in]`
  (most dialogs, e.g. Field Options, Edit Relationship, Specify Fields for Value List, have
  automation ids such as `IDC_LOOKUP_FIELD_LIST`), `Open-FmValueLists`,
  `Add-FmScriptSteps <snippet> -Run`.
* `script_check.ps1`: `Test-FmScripts <file> [-Xml out.xml] [-Click x,y]`
  checks a file whose scripts BaseCraft wrote: opens a copy, reports the
  damage dialog if any, lists every step as FileMaker prints it (Save a
  Copy as XML, `StepText`), and after a click (a button, or a layout
  change that fires a trigger) reads the custom dialog a script showed.
  `cargo run -p fmp12 --example scriptedit -- <Tasks en.fmp12> <out> edit,flow,calc,inplace`
  builds the file it was written for (`-Click 50,160`).
* `compile_snippet.py exprs out_prefix` + `compile_exprs.txt`: compiled-form
  oracle. Builds scripts of Set Variable steps; pasted with `Add-FmScriptSteps`
  into a file, FileMaker stores each calculation's key 5 and key 6, listed by
  `cargo run -p fmp12 --example calcdump -- <file>`
  (`crates/fmp12/tests/fixtures/compiled.tsv`). Paste into a file whose
  formats match the UI: in a French UI, decimal literals need a comma.
* More `fmp12` examples: `entedit` (delete or add raw entries of a file, for
  bisecting what FileMaker needs), `namekeys` (name-index keys next to the
  names), `schemaops` / `schemademo` (schema edits), `mkblank` (makes
  `assets/blank.fmp12` from a new file FileMaker created).
* `calc_oracle.ps1` + `calc_snippet.py`: calculation oracle. `Invoke-FmCalcs exprs.txt out.tsv`
  (or `Invoke-FmCalcBatches` for long files, 150 lines per script) pastes a script that
  `Evaluate`s each line and stores `index TAB result TAB EvaluationError` in
  `Tasks::Description` of a new record, runs it from the Script Workspace and reads the field
  back through UI Automation. Run it in a copy of `DynamicTemplates\Tasks\en.fmp12` (en_US
  formats) opened with `Start-Fm`. Expressions that produce lone UTF-16 surrogates or
  control characters can garble the field; wrap them in `Code ( ... )`. The results feed
  `crates/calc/tests/oracle.tsv`.
* `summary_demo.sh <out.fmp12> <basecraft-cli> <template.fmp12>`: a Sales table with every
  summary option (running, restarting, weighted, subtotaled, by population) and `GetSummary`
  calculations, in a copy of a template; `cargo run -p fmp12 --example fieldsonlayout --
  <out.fmp12> Sales <shown.fmp12>` puts its fields on its layout (styled like an edit box of the
  template, which FileMaker needs to show numbers). Open it, choose the Sales layout, Table
  view, and read the values through UI Automation (`AutomationId` `Field: Sales::…`, value
  pattern), unsorted and after the Region column's menu › Sort ascending.
