# Checks a file whose layouts BaseCraft wrote, in a running FileMaker Pro.
#   . tools/oracle/fmui.ps1; . tools/oracle/security_check.ps1; . tools/oracle/layout_check.ps1
#   Test-FmLayout <file.fmp12> -Layouts "BaseCraft Demo:3,Contact List 2:2" [-Xml out.xml]
#     [-Clicks "Objects Demo:520,300;140,600"] [-Keys "Objects Demo:{TAB}{TAB}"] [-NoRecord]
# Works on a copy (<file>-check.fmp12). FileMaker is shared: it waits until
# no FileMaker runs (Wait-FmFree) and quits only the FileMaker it started.
# For each layout (name:n = the n-th top-level entry of the layout pop-up),
# it takes a screenshot of FileMaker's window in Browse mode (after creating
# a record unless -NoRecord), then, for that layout, after each click of
# -Clicks (window coordinates, `;`-separated) and after the keys of -Keys
# (SendKeys syntax: {TAB}, +{TAB}…), then in Layout mode. Reports, as one
# object:
#   Open    : "OPEN" or the text of the dialog FileMaker shows instead;
#   Shots   : the screenshots (FileMaker's window only);
#   Xml     : with -Xml, Tools › Save a Copy as XML of the copy (or use
#             tools/oracle/fms_check.sh <file> Admin "" out.xml, headless).
# `BASECRAFT_LAYOUT_DEMO=<out.fmp12> cargo test -p basecraft layout_demo_file`
# builds the file it was written for from the Contacts starter template;
# `BASECRAFT_OBJECTS_DEMO=<out.fmp12> cargo test -p basecraft objects_demo_file`
# the "Objects Demo" layout of the Assets template (formats, conditional
# formats, popover, slide control, button bar, web viewer, tab order).

# Screenshot of FileMaker's main window (PrintWindow: nothing else on screen).
function Save-FmMainShot([string]$name) {
  $w = Get-FmWindows | ? { $_.Current.ClassName -like 'FMPRO*' } | Select -First 1
  $h = [IntPtr]$w.Current.NativeWindowHandle
  $r = New-Object W32+RECT; [void][W32]::GetWindowRect($h, [ref]$r)
  $bmp = New-Object Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
  [void][W32]::PrintWindow($h, $hdc, 2); $g.ReleaseHdc($hdc)
  $f = "$Shot\$name.png"; $bmp.Save($f); $f
}

# Answers FileMaker's "save changes to the layout?" with $button, if asked.
function Close-FmLayoutQuestion([string]$button = "Non") {
  Start-Sleep 1
  if (Find-Fm "Enregistrer" "Button") { Invoke-Fm $button; Start-Sleep 1 }
}

# Per-layout option of -Clicks / -Keys ("Layout:value|Layout:value").
function Get-FmLayoutOption([string]$spec, [string]$name) {
  foreach ($e in ($spec -split '\|')) {
    $n, $v = $e -split ':', 2
    if ($n -eq $name) { return $v }
  }
}

function Test-FmLayout([string]$file, [string]$Layouts, [string]$Xml, [string]$Clicks, [string]$Keys, [switch]$NoRecord) {
  $copy = [IO.Path]::ChangeExtension($file, $null).TrimEnd('.') + "-check.fmp12"
  Wait-FmFree
  Copy-Item $file $copy -Force
  $out = [ordered]@{ Copy = $copy; Open = $null; Shots = @(); Xml = $null }
  $script:MyFm = Start-Process "C:\Program Files\FileMaker\FileMaker Pro\FileMaker Pro.exe" -ArgumentList "`"$copy`"" -PassThru
  for ($i = 0; $i -lt 60 -and -not $out.Open; $i++) {
    Start-Sleep -Milliseconds 500
    $ws = @(Get-FmWindows)
    if ($ws | ? { $_.Current.Name -like 'Licence*' }) { Start-Sleep 1; Invoke-Fm "Continuer"; continue }
    $d = $ws | ? { $_.Current.Name -eq 'FileMaker Pro' }
    if ($d) { $out.Open = "DIALOG: " + (($d.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ? { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Text' } | % { $_.Current.Name }) -join ' ') }
    elseif ($ws | ? { $_.Current.ClassName -like 'FMPRO*' -and $_.Current.Name }) { Start-Sleep 2; $out.Open = "OPEN" }
  }
  if ($out.Open -ne "OPEN") { Stop-MyFm; return [pscustomobject]$out }
  $i = 0
  foreach ($entry in $Layouts.Split(',')) {
    $name, $n = $entry.Split(':')
    # The layout pop-up of the status toolbar: top-level entries, 20 px apart.
    Click-Fm 130 123; Start-Sleep 1
    Click-Fm 128 (145 + 20 * ([int]$n - 1)); Start-Sleep 2
    $tag = ($name -replace '\W', '_')
    if ($i -eq 0 -and -not $NoRecord) { Click-Fm 471 80; Start-Sleep 2 }   # a record to show
    $out.Shots += Save-FmMainShot "$tag-browse"
    $k = 0
    foreach ($c in ((Get-FmLayoutOption $Clicks $name) -split ';' | ? { $_ })) {
      $x, $y = $c -split ','
      Click-Fm ([int]$x) ([int]$y); Start-Sleep 2
      $out.Shots += Save-FmMainShot "$tag-click$k"; $k++
    }
    $keys = Get-FmLayoutOption $Keys $name
    if ($keys) {
      Send-FmKeys $keys; Start-Sleep 2
      $out.Shots += Save-FmMainShot "$tag-keys"
    }
    Click-Fm 995 123; Start-Sleep 3                    # Edit Layout
    $out.Shots += Save-FmMainShot "$tag-layout"
    Click-Fm 1268 123; Close-FmLayoutQuestion          # Exit Layout
    $i++
  }
  if ($Xml) { Export-FmXml $Xml; $out.Xml = $Xml }
  Stop-MyFm
  [pscustomobject]$out
}
