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
    $main = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' -and $_.Current.Name }
    if ($main -and -not (Get-FmLoginDialog)) { return "OPEN" }
    Start-Sleep -Milliseconds 500
  }
  "TIMEOUT"
}

# Launches FileMaker on a file and waits for its first window (login dialog
# or document); dismisses the trial dialog.
function Start-FmLogin([string]$file) {
  Stop-Fm
  Start-Process "C:\Program Files\FileMaker\FileMaker Pro\FileMaker Pro.exe" -ArgumentList "`"$file`""
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
    Stop-Fm
    $out += [pscustomobject]$r
  }
  $out
}
