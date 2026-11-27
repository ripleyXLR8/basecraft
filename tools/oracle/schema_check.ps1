# Checks a file whose schema BaseCraft edited, in a running FileMaker Pro.
#   . tools/oracle/fmui.ps1; . tools/oracle/schema_check.ps1
#   Test-FmSchema <file.fmp12> <Table> [-Records 2] [-Layout 2] [-Xml out.xml]
# Works on a copy (<file>-check.fmp12). Reports, as one object:
#   Open    : "OPEN" or the text of the dialog FileMaker shows instead;
#   Tables  : rows of Manage Database › Tables;
#   Fields  : rows of Manage Database › Fields for <Table> (name, type, options);
#   Records : number of records FileMaker created on <Table>'s layout (the
#             -Layout-th entry of the layout pop-up, 1-based);
#   Shots   : screenshots of the Manage Database dialog (FileMaker windows only).
# Read the created records back with `basecraft-cli records <copy> <Table>`.
# With -Xml, the copy is also saved as XML (Tools › Save a Copy as XML);
# compare two exports with xmldiff.py.
function Test-FmSchema([string]$file, [string]$table, [int]$Records = 2, [int]$Layout = 2, [string]$Xml) {
  $copy = [IO.Path]::ChangeExtension($file, $null).TrimEnd('.') + "-check.fmp12"
  Copy-Item $file $copy -Force
  $open = Test-FmOpen $copy
  $out = [ordered]@{ Copy = $copy; Open = $open; Tables = @(); Fields = @(); Records = 0; Shots = @() }
  if ($open -ne "OPEN") { return [pscustomobject]$out }
  Start-Fm $copy
  $d = Open-FmManageDb
  Select-FmTab "Tables"
  $out.Tables = @(Get-FmDbRows)
  $out.Shots += Save-FmWindowShot $d "$table-tables"
  # Select the table, then its fields; End scrolls the (virtual) list.
  $row = $d.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
    ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.DataItem' -and $_.Current.Name -like "Réorganiser $table *" } | Select -First 1
  $b = $row.Current.BoundingRectangle; [W32]::Click([int]($b.X + 60), [int]($b.Y + $b.Height / 2)); Start-Sleep 1
  Select-FmTab "Rubriques"
  $fields = @(Get-FmDbRows)
  $out.Shots += Save-FmWindowShot $d "$table-fields"
  $first = $d.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
    ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.DataItem' } | Select -First 1
  if ($first) {
    $b = $first.Current.BoundingRectangle; [W32]::Click([int]($b.X + 150), [int]($b.Y + $b.Height / 2)); Start-Sleep -Milliseconds 500
    [System.Windows.Forms.SendKeys]::SendWait("{END}"); Start-Sleep 1
    $fields += @(Get-FmDbRows)
    $out.Shots += Save-FmWindowShot $d "$table-fields-end"
  }
  $out.Fields = @($fields | Select -Unique)
  Invoke-Fm "OK"; Start-Sleep 2
  # The table's layout, from the layout pop-up of the status toolbar.
  $m = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' } | Select -First 1
  [W32]::Front([IntPtr]$m.Current.NativeWindowHandle); Start-Sleep -Milliseconds 500
  Click-Fm 130 123; Start-Sleep 1
  Click-Fm 104 (145 + 20 * ($Layout - 1)); Start-Sleep 1
  for ($i = 0; $i -lt $Records; $i++) { Click-Fm 471 80; Start-Sleep 1 }
  Click-Fm 500 500; Start-Sleep 1
  $out.Records = $Records
  $out.Shots += Save-FmShot "$table-browse"
  if ($Xml) { Export-FmXml $Xml }
  Close-Fm
  [pscustomobject]$out
}
