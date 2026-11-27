# Checks a file whose scripts BaseCraft wrote, in a running FileMaker Pro.
#   . tools/oracle/fmui.ps1; . tools/oracle/script_check.ps1
#   cargo run -p fmp12 --example scriptedit -- <Tasks en.fmp12> out.fmp12 "edit,delete,flow,calc,call,rename:Sort by Name:BC Renamed"
#   Test-FmScripts out.fmp12 -Xml out.xml -Click 50,160
# Works on a copy (<file>-check.fmp12). Reports, as one object:
#   Open   : "OPEN" or the text of the dialog FileMaker shows instead;
#   Steps  : every step as FileMaker prints it (Save a Copy as XML, StepText),
#            as objects { Script; Index; Text };
#   Scripts: the script list of the export (ScriptCatalog), in order;
#   Dialog : after a click at -Click x,y in the main window (e.g. a button
#            or a layout change that fires a script trigger), or after
#            choosing script -Menu n (1 = first script) in the Scripts menu,
#            the title and texts of the dialog a script showed.
# With the Tasks template and `call`, -Click 50,160 ("< Tasks") enters Task
# List, whose OnLayoutEnter runs "Sort by Status", which performs the new
# script BC Calc: the dialog reads 21|two|yes|two|x¶y (`inplace` puts the
# BC Calc steps in Sort by Status instead).
# A file from `basecraft-cli new-file` with scripts added, renamed and
# deleted (scriptedit ops beep, say, rename:…, drop:…): -Menu 2 runs the
# second script ("BC New ran"). Checked in FileMaker 26 (Nov 2026): both
# open with no warning ([17].[1] key 8 written, FORMAT.md §8).

# Step texts of a Save-a-Copy-as-XML export.
function Get-FmStepText([string]$xml) {
  $x = [xml](Get-Content $xml -Raw -Encoding utf8)
  $text = @{}
  foreach ($n in $x.SelectNodes('//*[@datatype="StepText"]')) { $text[$n.LocalName.TrimStart('_')] = $n.InnerText }
  foreach ($s in $x.FMSaveAsXML.Structure.AddAction.StepsForScripts.Script) {
    foreach ($st in $s.ObjectList.Step) {
      [pscustomobject]@{ Script = $s.ScriptReference.name; Index = [int]$st.index; Text = $text[$st.UUID] }
    }
  }
}

# Title and texts of the dialog over FileMaker's main window, if any.
function Get-FmDialog {
  $w = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' } | Select -First 1
  $d = $w.FindAll($TS::Children, [System.Windows.Automation.Condition]::TrueCondition) |
    ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' } | Select -First 1
  if (-not $d) { return $null }
  $texts = $d.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
    ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $_.Current.Name } | % { $_.Current.Name }
  [pscustomobject]@{ Title = $d.Current.Name; Texts = @($texts); Shot = (Save-FmWindowShot $d "dialog-$($d.Current.Name)") }
}

# Runs script n (1-based) of the Scripts menu ("Scripts" at 408,40 in the
# French UI; its first item is the Script Workspace). The Codejock menu is
# invisible to UI Automation, so the keyboard walks it.
function Invoke-FmScriptMenu([int]$n) {
  Click-Fm 408 40; Start-Sleep 1
  [System.Windows.Forms.SendKeys]::SendWait(("{DOWN}" * ($n + 1)) + "{ENTER}")
}

function Test-FmScripts([string]$file, [string]$Xml, [int[]]$Click, [int]$Menu) {
  $copy = [IO.Path]::ChangeExtension($file, $null).TrimEnd('.') + "-check.fmp12"
  Copy-Item $file $copy -Force
  $out = [ordered]@{ Copy = $copy; Open = (Test-FmOpen $copy); Steps = @(); Scripts = @(); Dialog = $null }
  if ($out.Open -ne "OPEN") { return [pscustomobject]$out }
  Start-Fm $copy
  if ($Xml) {
    Export-FmXml $Xml; $out.Steps = @(Get-FmStepText $Xml)
    $x = [xml](Get-Content $Xml -Raw -Encoding utf8)
    $out.Scripts = @($x.FMSaveAsXML.Structure.AddAction.ScriptCatalog.Script | % { $_.name })
  }
  if ($Click -or $Menu) {
    if ($Click) { Click-Fm $Click[0] $Click[1] } else { Invoke-FmScriptMenu $Menu }
    Start-Sleep 3
    $out.Dialog = Get-FmDialog
    if ($out.Dialog) { [System.Windows.Forms.SendKeys]::SendWait("{ENTER}"); Start-Sleep 1 }
  }
  Close-Fm
  [pscustomobject]$out
}
