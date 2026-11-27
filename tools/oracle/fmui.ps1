# Helpers to drive FileMaker Pro via UI Automation. Dot-source this file.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W32 {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
  public static void Click(int x,int y){ SetCursorPos(x,y); mouse_event(2,0,0,0,UIntPtr.Zero); mouse_event(4,0,0,0,UIntPtr.Zero); }
  public static void Front(IntPtr h){ keybd_event(0x12,0,0,UIntPtr.Zero); keybd_event(0x12,0,2,UIntPtr.Zero); ShowWindow(h,9); SetForegroundWindow(h); }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
  public struct RECT { public int L, T, R, B; }
}
"@
$AE = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$Shot = Join-Path $env:TEMP "openmaker-shots"
New-Item -ItemType Directory -Force $Shot | Out-Null

function Get-FmWindows {
  $p = Get-Process "FileMaker Pro" -ErrorAction SilentlyContinue | Select -First 1
  if (-not $p) { return @() }
  $c = New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $p.Id)
  $AE::RootElement.FindAll($TS::Children, $c)
}

function Find-Fm([string]$name, [string]$type) {
  foreach ($w in Get-FmWindows) {
    foreach ($e in $w.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
      if ($e.Current.Name -eq $name -and (-not $type -or $e.Current.ControlType.ProgrammaticName -like "*$type")) { return $e }
    }
  }
}

function Invoke-Fm([string]$name, [string]$type = "Button") {
  $e = Find-Fm $name $type
  if (-not $e) { throw "not found: $name" }
  $pat = $null
  if ($e.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pat)) { $pat.Invoke() }
  elseif ($e.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$pat)) { $pat.Expand() }
  else { throw "not invokable: $name" }
}

function Show-FmTree([int]$max = 200) {
  foreach ($w in Get-FmWindows) {
    "WINDOW: $($w.Current.Name) [$($w.Current.ClassName)]"
    $i = 0
    foreach ($e in $w.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
      if ($i++ -ge $max) { break }
      "  $($e.Current.ControlType.ProgrammaticName -replace 'ControlType.','') | $($e.Current.Name) | $($e.Current.AutomationId)"
    }
  }
}

# Screenshot of FileMaker windows only (never the whole desktop).
function Save-FmShot([string]$name) {
  $i = 0
  foreach ($w in Get-FmWindows) {
    $h = [IntPtr]$w.Current.NativeWindowHandle
    $r = New-Object W32+RECT; [void][W32]::GetWindowRect($h, [ref]$r)
    $bmp = New-Object Drawing.Bitmap ([Math]::Max(1,$r.R - $r.L)), ([Math]::Max(1,$r.B - $r.T))
    $g = [Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
    [void][W32]::PrintWindow($h, $hdc, 2); $g.ReleaseHdc($hdc)
    $f = "$Shot\$name-$i.png"; $bmp.Save($f); $f; $i++
  }
}

function Send-FmKeys([string]$keys) {
  $w = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' -or $_.Current.ClassName -eq '#32770' } | Select -First 1
  [W32]::Front([IntPtr]$w.Current.NativeWindowHandle)
  Start-Sleep -Milliseconds 300
  [System.Windows.Forms.SendKeys]::SendWait($keys)
}

# Screen capture of the main FileMaker window rectangle (includes popup menus over it).
function Save-FmScreen([string]$name) {
  $w = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' } | Select -First 1
  $r = New-Object W32+RECT; [void][W32]::GetWindowRect([IntPtr]$w.Current.NativeWindowHandle, [ref]$r)
  $bmp = New-Object Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $f = "$Shot\$name.png"; $bmp.Save($f); $f
}

# Left click at (x, y) relative to the main FileMaker window.
function Click-Fm([int]$x, [int]$y) {
  $w = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' } | Select -First 1
  $r = New-Object W32+RECT; [void][W32]::GetWindowRect([IntPtr]$w.Current.NativeWindowHandle, [ref]$r)
  [W32]::Click($r.L + $x, $r.T + $y)
}

function Stop-Fm {
  Get-Process "FileMaker Pro" -ErrorAction SilentlyContinue | % { [void]$_.CloseMainWindow(); if (-not $_.WaitForExit(15000)) { $_.Kill() } }
}

# Launch FileMaker on a file and dismiss the trial dialog.
function Start-Fm([string]$file) {
  Start-Process "C:\Program Files\FileMaker\FileMaker Pro\FileMaker Pro.exe" -ArgumentList "`"$file`""
  for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    $ws = @(Get-FmWindows)
    if ($ws | ? { $_.Current.Name -like 'Licence*' }) { Start-Sleep 1; Invoke-Fm "Continuer"; continue }
    if ($ws | ? { $_.Current.ClassName -like 'FMPRO*' -and $_.Current.Name }) { Start-Sleep 2; return }
  }
  throw "FileMaker did not open $file"
}

function Set-FmCheck([string]$n, [bool]$on) {
  $c = Find-Fm $n "CheckBox"; $t = $c.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
  if (($t.Current.ToggleState -eq 'On') -ne $on) {
    try { $t.Toggle() } catch { $b = $c.Current.BoundingRectangle; [W32]::Click([int]($b.X + 8), [int]($b.Y + $b.Height / 2)); Start-Sleep -Milliseconds 300 }
  }
}

# Tools > Save a Copy as XML of the frontmost file, into $out (full path).
function Export-FmXml([string]$out) {
  Click-Fm 455 40; Start-Sleep -Milliseconds 700; Click-Fm 560 163; Start-Sleep 2
  Set-FmCheck "Inclure les détails pour les outils d'analyse" $true
  Set-FmCheck "Enregistrer les données binaires de chaque objet de modèle sous son nœud" $true
  Set-FmCheck "Ouvrir automatiquement le récapitulatif après sa création" $false
  Invoke-Fm "Créer"; Start-Sleep 2
  $d = Find-Fm "Enregistrer le rapport" "Window"
  [W32]::Front([IntPtr]$d.Current.NativeWindowHandle); Start-Sleep -Milliseconds 400
  [System.Windows.Forms.SendKeys]::SendWait("%n"); Start-Sleep -Milliseconds 200
  [System.Windows.Forms.SendKeys]::SendWait(($out -replace '([+^%~(){}\[\]])','{$1}') + "{ENTER}")
  for ($i = 0; $i -lt 60 -and -not (Test-Path $out); $i++) { Start-Sleep -Milliseconds 500 }
  Start-Sleep 2
}

# Opens a file and reports "OPEN" or the text of the blocking dialog.
function Test-FmOpen([string]$file) {
  Stop-Fm
  Start-Process "C:\Program Files\FileMaker\FileMaker Pro\FileMaker Pro.exe" -ArgumentList "`"$file`""
  for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    $ws = @(Get-FmWindows)
    if ($ws | ? { $_.Current.Name -like 'Licence*' }) { Start-Sleep 1; Invoke-Fm "Continuer"; continue }
    if ($ws | ? { $_.Current.ClassName -like 'FMPRO*' -and $_.Current.Name }) { $r = "OPEN"; break }
    $d = $ws | ? { $_.Current.Name -eq 'FileMaker Pro' -and $_.Current.ClassName -eq 'Window' }
    if ($d) { Start-Sleep 1; $t = ($d.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $_.Current.Name.Length -gt 10 } | Select -First 1).Current.Name; $r = "DIALOG: $t"; break }
  }
  if (-not $r) { $r = "TIMEOUT" }
  Stop-Fm
  $r
}
