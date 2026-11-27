# Evaluates calculation expressions in a running FileMaker Pro.
#   . tools/oracle/fmui.ps1; . tools/oracle/calc_oracle.ps1
#   Start-Fm <copy of DynamicTemplates\Tasks\en.fmp12>
#   Invoke-FmCalcs exprs.txt out.tsv
# exprs.txt: one expression per line. out.tsv: expression TAB result
# [TAB error code when EvaluationError is not 0]. The result is escaped
# (\\ \t \r \n). Each call pastes a new script (calc_snippet.py) into the
# Script Workspace, runs it, and reads Tasks::Description of the record it
# created through UI Automation (Task Details layout).
function Invoke-FmCalcs([string]$exprs, [string]$out) {
  $xml = [IO.Path]::ChangeExtension($out, ".xml")
  python "$PSScriptRoot\calc_snippet.py" $exprs $xml
  $sw = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
  if (-not $sw) {
    Click-Fm 408 40; Start-Sleep 1; Click-Fm 480 63; Start-Sleep 3
    $sw = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
  }
  $h = [IntPtr]$sw.Current.NativeWindowHandle
  $r = New-Object W32+RECT; [void][W32]::GetWindowRect($h, [ref]$r)
  [W32]::Front($h); [W32]::Click($r.L + 56, $r.T + 75); Start-Sleep 2   # New Script
  Set-FmClip "Mac-XMSS" (Get-Content $xml -Raw -Encoding utf8)
  [W32]::Click($r.L + 600, $r.T + 160); Start-Sleep 1
  [System.Windows.Forms.SendKeys]::SendWait("^v"); Start-Sleep 3
  [System.Windows.Forms.SendKeys]::SendWait("^s"); Start-Sleep 2
  Invoke-Fm "Exécuter"
  $src = @(Get-Content $exprs -Encoding utf8 | ? { $_.Trim() -and -not $_.StartsWith('#') })
  $val = ""
  for ($i = 0; $i -lt 240 -and -not ($val -match "(^|\n|\r)$($src.Count)`t[^\r\n]*`t\d+\s*$"); $i++) {
    Start-Sleep -Milliseconds 500
    foreach ($w in Get-FmWindows) {
      $c = New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, 'Field: Tasks::Description')
      $e = $w.FindFirst($TS::Descendants, $c)
      if ($e) { $val = $e.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value }
    }
  }
  $lines = foreach ($l in ($val -split "`r?`n|`r")) {
    if (-not $l) { continue }
    $f = $l -split "`t"
    $row = $src[[int]$f[0] - 1] + "`t" + $f[1]
    if ($f[2] -ne '0') { $row += "`t" + $f[2] }
    $row
  }
  [IO.File]::WriteAllLines($out, [string[]]$lines, (New-Object Text.UTF8Encoding $false))
  "$($lines.Count) / $($src.Count) results -> $out"
}

# Runs a long expression file in chunks of $size lines (one script each) and
# concatenates the results.
function Invoke-FmCalcBatches([string]$exprs, [string]$out, [int]$size = 150) {
  $src = @(Get-Content $exprs -Encoding utf8 | ? { $_.Trim() -and -not $_.StartsWith('#') })
  $all = @()
  for ($i = 0; $i -lt $src.Count; $i += $size) {
    $part = [IO.Path]::ChangeExtension($out, ".part$i.txt")
    $src[$i..([Math]::Min($i + $size, $src.Count) - 1)] | Set-Content -Encoding utf8 $part
    Invoke-FmCalcs $part "$part.tsv" | Out-Null
    $all += Get-Content "$part.tsv" -Encoding utf8
  }
  [IO.File]::WriteAllLines($out, [string[]]$all, (New-Object Text.UTF8Encoding $false))
  "$($all.Count) / $($src.Count) results -> $out"
}
