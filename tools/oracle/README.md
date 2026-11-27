# Oracle tooling

Scripts that drive a locally installed, licensed (or trial) FileMaker Pro 26
on Windows to produce **ground truth** for OpenMaker's tests. Nothing here
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
* `mutate.py`, `xmldiff.py`, `attrib_fields.py`: reverse oracle. Flip bits in copies, export XML, attribute the differences.
* `token_sweep_analyze.py`: reads the XML of files whose calculation token bytes were overwritten (one way calc_tables.rs was built).
* `idxdec.py`: decodes a root index sector next to the first chunks of its children.
