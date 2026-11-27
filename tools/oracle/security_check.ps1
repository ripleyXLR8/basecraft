# Checks a file whose accounts and privilege sets BaseCraft wrote, in a
# running FileMaker Pro 26 (French UI).
#   . tools/oracle/fmui.ps1; . tools/oracle/security_check.ps1
#   cargo run -p fmp12 --example secdemo -- <Tasks en.fmp12> sec.fmp12
#   Test-FmSecurity sec.fmp12 -Logins "alice:test123,alice:wrong,bob:test123,Admin:test123" -Xml sec.xml
# Works on a copy (<file>-check.fmp12). For each account:password it opens
# the copy, reports what FileMaker shows (the login dialog, then "OPEN" or
# the alert text), and screenshots FileMaker's window. The first login that
# opens with full access saves the copy as XML (-Xml), whose
# PrivilegeSetsCatalog / AccountsCatalog / ExtendedPrivilegesCatalog list
# what FileMaker read (Get-FmSecurityXml summarises them).

# The login dialog ("Ouvrir <file>"), if FileMaker shows one.
function Get-FmLoginDialog { Find-FmWindow "Ouvrir*" }

# Fills the login dialog and signs in. Returns "OPEN", or the text of the
# alert FileMaker shows instead (wrong password, inactive account…).
function Submit-FmLogin([string]$account, [string]$password) {
  $d = Get-FmLoginDialog
  if (-not $d) { return "NO LOGIN DIALOG" }
  foreach ($p in @(@("accountNameBox", $account), @("passwordBox", $password))) {
    $c = New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, $p[0])
    $e = $d.FindFirst($TS::Descendants, $c)
    $e.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($p[1])
  }
  Click-FmId "IDSIGNIN" $d; Start-Sleep 3
  for ($i = 0; $i -lt 20; $i++) {
    $texts = @()
    foreach ($w in Get-FmWindows) {
      foreach ($x in @($w) + @($w.FindAll($TS::Children, [System.Windows.Automation.Condition]::TrueCondition))) {
        if ($x.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' -and $x.Current.Name -eq 'FileMaker Pro') {
          $texts += $x.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
            ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $_.Current.Name.Length -gt 10 } | % { $_.Current.Name }
          [void](Save-FmWindowShot $x "login-alert-$account")
        }
      }
    }
    if ($texts) { return "ALERT: " + ($texts -join ' ') }
    # Another dialog (e.g. the change of password an account must make).
    $other = Get-FmWindows | % { @($_) + @($_.FindAll($TS::Children, [System.Windows.Automation.Condition]::TrueCondition)) } |
      ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' -and $_.Current.Name -and $_.Current.Name -notlike 'Ouvrir*' -and $_.Current.ClassName -notlike 'FMPRO*' -and $_.Current.Name -ne 'FileMaker Pro' } |
      Select -First 1
    if ($other -and $i -gt 4) {
      [void](Save-FmWindowShot $other "login-dialog-$account")
      $t = ($other.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ? { $_.Current.Name } | % { $_.Current.Name }) -join ' | '
      return "DIALOG: $($other.Current.Name): $t"
    }
    $main = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' -and $_.Current.Name }
    if ($main -and -not (Get-FmLoginDialog)) { return "OPEN" }
    Start-Sleep -Milliseconds 500
  }
  "TIMEOUT"
}

# FileMaker is shared with other agents: wait until none is running (up to
# $minutes), never stop one this script did not start.
function Wait-FmFree([int]$minutes = 30) {
  for ($i = 0; $i -lt $minutes * 12 -and (Get-Process "FileMaker Pro" -ErrorAction SilentlyContinue); $i++) { Start-Sleep 5 }
  if (Get-Process "FileMaker Pro" -ErrorAction SilentlyContinue) { throw "FileMaker is busy" }
}

# Quits the FileMaker this script started (dialogs dismissed first).
function Stop-MyFm {
  if (-not $script:MyFm) { return }
  foreach ($w in Get-FmWindows) { if ($w.Current.Name -eq 'FileMaker Pro') { try { Invoke-Fm "OK" } catch {} } }
  $p = Get-Process -Id $script:MyFm.Id -ErrorAction SilentlyContinue
  if ($p) { [void]$p.CloseMainWindow(); if (-not $p.WaitForExit(20000)) { $p.Kill() } }
  $script:MyFm = $null
}

# Launches FileMaker on a file and waits for its first window (login dialog
# or document); dismisses the trial dialog.
function Start-FmLogin([string]$file) {
  Wait-FmFree
  $script:MyFm = Start-Process "C:\Program Files\FileMaker\FileMaker Pro\FileMaker Pro.exe" -ArgumentList "`"$file`"" -PassThru
  for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    $ws = @(Get-FmWindows)
    if ($ws | ? { $_.Current.Name -like 'Licence*' }) { Start-Sleep 1; Invoke-Fm "Continuer"; continue }
    if (Get-FmLoginDialog) { Start-Sleep 1; return "LOGIN" }
    $d = $ws | ? { $_.Current.Name -eq 'FileMaker Pro' }
    if ($d) { return "DIALOG: " + (($d.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' } | % { $_.Current.Name }) -join ' ') }
    if ($ws | ? { $_.Current.ClassName -like 'FMPRO*' -and $_.Current.Name } ) { Start-Sleep 2; if (-not (Get-FmLoginDialog)) { return "OPEN" } }
  }
  "TIMEOUT"
}

# Accounts, privilege sets and extended privileges of a Save-a-Copy-as-XML export.
function Get-FmSecurityXml([string]$xml) {
  $x = [xml](Get-Content $xml -Raw -Encoding utf8)
  $add = $x.FMSaveAsXML.Structure.AddAction
  [pscustomobject]@{
    Accounts = @($add.AccountsCatalog.Account | % { "$($_.Authentication.AccountName) | $($_.PrivilegeSetReference.name) | $($_.Authentication.type) | enabled=$($_.Authentication.status) | $($_.Authentication.changePasswordOnNextLogin)" })
    Sets     = @($add.PrivilegeSetsCatalog.PrivilegeSet | % { $_.OuterXml })
    Extended = @($add.ExtendedPrivilegesCatalog.ExtendedPrivilege | % { "$($_.name): " + (($_.ObjectList.PrivilegeSetReference | % { $_.name }) -join ', ') })
  }
}

function Test-FmSecurity([string]$file, [string]$Logins, [string]$Xml) {
  $copy = [IO.Path]::ChangeExtension($file, $null).TrimEnd('.') + "-check.fmp12"
  $out = @()
  $exported = $false
  foreach ($pair in $Logins.Split(',')) {
    $acc, $pw = $pair.Split(':', 2)
    # A fresh copy each time (FileMaker changes the files it opens).
    Wait-FmFree
    Copy-Item $file $copy -Force
    $start = Start-FmLogin $copy
    $r = [ordered]@{ Login = $acc; Start = $start; Result = $null; Shot = $null }
    if ($start -eq "LOGIN") {
      $r.Result = Submit-FmLogin $acc $pw
      if ($r.Result -eq "OPEN") {
        Start-Sleep 2
        $r.Shot = Save-FmShot "security-$acc"
        if ($Xml -and -not $exported -and $acc -eq 'Admin') { Export-FmXml $Xml; $exported = $true }
      }
    }
    Stop-MyFm
    $out += [pscustomobject]$r
  }
  $out
}

# Has FileMaker itself run the account script steps of account_steps.xml
# in a copy of a file that logs in as Admin by itself (blank.fmp12): Add
# Account (FileMaker hashes the test passwords), Enable Account, Reset
# Account Password, Delete Account, Re-Login, Change Password. Returns the
# text of the closing BCRESULT dialog (the Get ( LastError ) of each step)
# and leaves the file for BaseCraft to read (passwords, digests, steps).
#   Invoke-FmAccountSteps $env:TEMP\steps.fmp12 tools\oracle\account_steps.xml
function Invoke-FmAccountSteps([string]$file, [string]$snippet) {
  Wait-FmFree
  Start-Fm $file
  Add-FmScriptSteps (Resolve-Path $snippet) -Run
  $text = $null
  for ($i = 0; $i -lt 30 -and -not $text; $i++) {
    $d = Find-FmWindow "BCRESULT"
    if ($d) {
      $text = ($d.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
        ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' } | % { $_.Current.Name }) -join ' '
      [void](Save-FmWindowShot $d "account-steps")
      Invoke-Fm "OK"
    }
    Start-Sleep 1
  }
  Close-Fm
  $text
}
