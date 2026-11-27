# Checks a file whose scripts BaseCraft wrote, in a running FileMaker Pro.
#   . tools/oracle/fmui.ps1; . tools/oracle/script_check.ps1
#   cargo run -p fmp12 --example scriptedit -- <Tasks en.fmp12> out.fmp12 edit,flow,calc,inplace
#   Test-FmScripts out.fmp12 -Xml out.xml -Click 50,160
# Works on a copy (<file>-check.fmp12). Reports, as one object:
#   Open   : "OPEN" or the text of the dialog FileMaker shows instead;
#   Steps  : every step as FileMaker prints it (Save a Copy as XML, StepText),
#            as objects { Script; Index; Text };
#   Dialog : after a click at -Click x,y in the main window (e.g. a button
#            or a layout change that fires a script trigger), the title and
#            texts of the dialog a script showed (Show Custom Dialog).
# With the Tasks template and `inplace`, -Click 50,160 ("< Tasks") enters
# Task List, whose OnLayoutEnter runs "Sort by Status" (the BC Calc steps):
# the dialog reads 21|two|yes|two|x¶y.

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

function Test-FmScripts([string]$file, [string]$Xml, [int[]]$Click) {
  $copy = [IO.Path]::ChangeExtension($file, $null).TrimEnd('.') + "-check.fmp12"
  Copy-Item $file $copy -Force
  $out = [ordered]@{ Copy = $copy; Open = (Test-FmOpen $copy); Steps = @(); Dialog = $null }
  if ($out.Open -ne "OPEN") { return [pscustomobject]$out }
  Start-Fm $copy
  if ($Xml) { Export-FmXml $Xml; $out.Steps = @(Get-FmStepText $Xml) }
  if ($Click) {
    Click-Fm $Click[0] $Click[1]; Start-Sleep 3
    $out.Dialog = Get-FmDialog
    if ($out.Dialog) { [System.Windows.Forms.SendKeys]::SendWait("{ENTER}"); Start-Sleep 1 }
  }
  Close-Fm
  [pscustomobject]$out
}
