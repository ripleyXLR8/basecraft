$ErrorActionPreference = "Continue"
$repo = (Resolve-Path "$PSScriptRoot\..\..\..").Path
. "$repo\tools\oracle\fmui.ps1"
. "$repo\tools\oracle\script_check.ps1"
. "$repo\tools\oracle\security_check.ps1"
# Work folder: copies, results and the data files the scripts write.
$D = if ($env:BC_DF_DIR) { $env:BC_DF_DIR } else { Join-Path $env:TEMP "basecraft-datafiles" }
New-Item -ItemType Directory -Force $D | Out-Null
# Only ever drive the FileMaker this script started.
function Get-FmWindows {
  if (-not $script:MyFm) { return @() }
  $c = New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $script:MyFm.Id)
  $AE::RootElement.FindAll($TS::Children, $c)
}
if (Test-Path "$D\mypid.txt") { $script:MyFm = Get-Process -Id ([int](Get-Content "$D\mypid.txt")) -ErrorAction SilentlyContinue }

function Start-Mine([string]$name) {
  Copy-Item "C:\Program Files\FileMaker\FileMaker Pro\DynamicTemplates\Tasks\en.fmp12" "$D\$name.fmp12" -Force
  $r = Start-FmLogin "$D\$name.fmp12"
  Set-Content "$D\mypid.txt" $script:MyFm.Id
  $r
}

function Stop-Mine { Stop-MyFm; Remove-Item "$D\mypid.txt" -ErrorAction SilentlyContinue }

function Open-Workspace {
  $sw = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
  for ($try = 0; $try -lt 4 -and -not $sw; $try++) {
    $m = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' } | Select -First 1
    [W32]::Front([IntPtr]$m.Current.NativeWindowHandle); Start-Sleep -Milliseconds 500
    Click-Fm 408 40; Start-Sleep 1; Click-Fm 480 63; Start-Sleep 3
    $sw = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
    if (-not $sw) { [System.Windows.Forms.SendKeys]::SendWait("{ESC}{ESC}"); Start-Sleep 1 }
  }
  $sw
}

# New script, paste $snippet, select all, copy back to $out.
function Copy-Back([string]$snippet, [string]$out) {
  $sw = Open-Workspace
  if (-not $sw) { [void](Save-FmScreen "df-noworkspace"); Get-FmWindows | % { "WIN " + $_.Current.Name }; return }
  "SW: " + $sw.GetType().FullName + " " + @($sw).Count + " " + (@($sw)[0].Current.Name) + " h=" + (@($sw)[0].Current.NativeWindowHandle)
  $sw = @($sw)[-1]
  $h = [IntPtr]$sw.Current.NativeWindowHandle
  $r = New-Object W32+RECT; [void][W32]::GetWindowRect($h, [ref]$r)
  "RECT " + $r.L + "," + $r.T + "," + $r.R + "," + $r.B
  [W32]::Front($h); [W32]::Click($r.L + 56, $r.T + 75); Start-Sleep 2
  Set-FmClip "Mac-XMSS" (Get-Content $snippet -Raw -Encoding utf8)
  [W32]::Click($r.L + 600, $r.T + 160); Start-Sleep 1
  [System.Windows.Forms.SendKeys]::SendWait("^v"); Start-Sleep 4
  [System.Windows.Forms.SendKeys]::SendWait("^s"); Start-Sleep 3
  $sw2 = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
  if ($sw2) { [void](Save-FmWindowShot $sw2 "df-sw-pasted") }
  [System.Windows.Forms.Clipboard]::SetText("empty")
  [W32]::Front($h); Start-Sleep -Milliseconds 500
  [W32]::Click($r.L + 500, $r.T + 160); Start-Sleep 1
  [System.Windows.Forms.SendKeys]::SendWait("^a"); Start-Sleep 1
  [W32]::keybd_event(0x11,0,0,[UIntPtr]::Zero); [W32]::keybd_event(0x43,0,0,[UIntPtr]::Zero)
  [W32]::keybd_event(0x43,0,2,[UIntPtr]::Zero); [W32]::keybd_event(0x11,0,2,[UIntPtr]::Zero)
  for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    if ([System.Windows.Forms.Clipboard]::ContainsData("Mac-XMSS")) { "copy after $i"; break }
  }
  $do = [System.Windows.Forms.Clipboard]::GetDataObject()
  "formats: " + ($do.GetFormats() -join ', ')
  $ms = $do.GetData("Mac-XMSS")
  if ($ms -and $out) {
    $b = $ms.ToArray()
    [IO.File]::WriteAllBytes($out, $b[4..($b.Length-1)])
    "copied " + $b.Length
  }
}

function Close-Workspace {
  $sw = Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' } | Select -First 1
  if ($sw) { $sw.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close(); Start-Sleep 2 }
  for ($i = 0; $i -lt 30 -and (Get-FmWindows | ? { $_.Current.Name -like 'Espace de travail*' }); $i++) {
    if (Find-Fm "Enregistrer tout" "Button") { Invoke-Fm "Enregistrer tout" }
    Start-Sleep 1
  }
}

function Get-Description {
  $c = New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, "Field: Tasks::Description")
  foreach ($w in Get-FmWindows) {
    foreach ($e in $w.FindAll($TS::Descendants, $c)) {
      $vp = $null
      if ($e.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$vp)) { $vp.Current.Value -replace "`r", "<CR>" -replace "`n", "<LF>" -replace "`0", "<NUL>" }
    }
  }
}
