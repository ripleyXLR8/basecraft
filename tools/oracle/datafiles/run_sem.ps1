# Runs script $script of <work folder>\$file.fmp12 (built by round.sh) in FileMaker Pro;
# prints Tasks::Description of the record the script made (its results).
param([string]$file = "sem5", [string]$script = "BC Steps")
. "$PSScriptRoot\lib.ps1"
Remove-Item "$D\bc_*" -Recurse -Force -ErrorAction SilentlyContinue
Wait-FmFree 60
Copy-Item "$D\$file.fmp12" "$D\run-$file.fmp12" -Force
"start: " + (Start-FmLogin "$D\run-$file.fmp12")
Set-Content "$D\mypid.txt" $script:MyFm.Id
Start-Sleep 5
$sw = @(Open-Workspace)[-1]
if (-not $sw) { "no workspace"; Stop-Mine; exit }
$c = New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $script)
$item = $sw.FindFirst($TS::Descendants, $c)
if (-not $item) { "no script item"; Save-FmWindowShot $sw "df-sem-noitem"; Stop-Mine; exit }
$b = $item.Current.BoundingRectangle
[W32]::Front([IntPtr]$sw.Current.NativeWindowHandle); Start-Sleep -Milliseconds 500
"ITEM " + $item.Current.ControlType.ProgrammaticName + " " + $b
$sp = $null; if ($item.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$sp)) { $sp.Select(); "selected" }
[W32]::Click([int]($b.X + $b.Width / 2), [int]($b.Y + $b.Height / 2)); Start-Sleep 2
[void](Save-FmWindowShot $sw "df-sem-selected")
Click-FmId "buttonRun" $sw; Start-Sleep 20
foreach ($w in Get-FmWindows) { "WIN " + $w.Current.Name }
$d = Get-FmDialog
if ($d) { "DIALOG: " + $d.Title + ": " + ($d.Texts -join ' | '); [System.Windows.Forms.SendKeys]::SendWait("{ENTER}"); Start-Sleep 1 }
Close-Workspace
Start-Sleep 2
"RESULT: " + ((Get-Description) -join ' || ')
[void](Save-FmShot "df-sem-run")
Get-ChildItem "$D\bc_*" | % { "FILE " + $_.Name + " " + $_.Length }
Stop-Mine
"done"
