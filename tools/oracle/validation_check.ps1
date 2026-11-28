# Validation codes FileMaker reports when a script commits a record.
#   . tools/oracle/fmui.ps1; . tools/oracle/security_check.ps1
#   . tools/oracle/validation_check.ps1
#   Test-FmValidation <dir> -Fields @(,@('R','--type','number','--range','1:10'))
#                     -Cases @('r:13:R:12')
# Copies the Tasks template to <dir>\val.fmp12, adds the fields with
# `basecraft-cli schema add-field` (their ids are printed; new fields start
# at 13), pastes a script built by validation_snippet.py (each case: New
# Record, Set Field id:name:calc [| more], Commit, Get ( LastError ), Revert
# unless the variable ends in +), runs it and reads Tasks::Description.
#
# FileMaker Pro 26.0.1 (Windows), checked with this script:
#   range (number, text, date), maximum characters and member of value list
#   are not checked, even "always": 0; validated by calculation 507 (not on
#   an empty value); unique 504; existing 505; not empty 509 for a field the
#   script never set; the first failing field by id is reported.
#   GetSummary (`?var:calc` cases, `!sort:id:name`, `!unsort`, `!first`,
#   `!next`): empty unsorted; by the break field when sorted (the empty
#   value's group too); a Total of no values is empty, a Count 0.
function Test-FmValidation([string]$dir, $Fields, [string[]]$Cases, [string]$Cli = "target\debug\basecraft-cli.exe") {
  $file = Join-Path $dir "val.fmp12"
  Copy-Item "C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates\Tasks\en.fmp12" $file -Force
  foreach ($a in $Fields) {
    $j = & $Cli schema add-field $file Tasks @a --in-place | ConvertFrom-Json
    "$($a[0])=$($j.result.added_field.id)"
  }
  python "$PSScriptRoot\validation_snippet.py" "$dir\val.xml" @Cases
  try {
    "start: " + (Start-FmLogin $file)
    Start-Sleep 3
    $sw = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
    if (-not $sw) {
      Click-Fm 408 40; Start-Sleep 1; Click-Fm 480 63; Start-Sleep 3
      $sw = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
    }
    $h = [IntPtr]$sw.Current.NativeWindowHandle
    $r = New-Object W32+RECT; [void][W32]::GetWindowRect($h, [ref]$r)
    [W32]::Front($h); [W32]::Click($r.L + 56, $r.T + 75); Start-Sleep 2
    Set-FmClip "Mac-XMSS" (Get-Content "$dir\val.xml" -Raw -Encoding utf8)
    [W32]::Click($r.L + 600, $r.T + 160); Start-Sleep 1
    [System.Windows.Forms.SendKeys]::SendWait("^v"); Start-Sleep 3
    [System.Windows.Forms.SendKeys]::SendWait("^s"); Start-Sleep 2
    Invoke-Fm "Exécuter"
    $val = ""
    for ($i = 0; $i -lt 120 -and -not ($val -match "END"); $i++) {
      Start-Sleep -Milliseconds 500
      foreach ($w in Get-FmWindows) {
        $c = New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, 'Field: Tasks::Description')
        $e = $w.FindFirst($TS::Descendants, $c)
        if ($e) { $val = $e.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value }
      }
    }
    "RESULT: $val"
  } finally {
    Stop-MyFm
  }
}
