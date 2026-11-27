# Checks a file whose layouts BaseCraft wrote, in a running FileMaker Pro.
#   . tools/oracle/fmui.ps1; . tools/oracle/layout_check.ps1
#   Test-FmLayout <file.fmp12> -Layouts "BaseCraft Demo:3,Contact List 2:2" [-Xml out.xml]
# Works on a copy (<file>-check.fmp12). For each layout (name:n = the n-th
# top-level entry of the layout pop-up), it takes a screenshot of
# FileMaker's window in Browse mode (after creating a record) and in
# Layout mode. Reports, as one object:
#   Open    : "OPEN" or the text of the dialog FileMaker shows instead;
#   Shots   : the screenshots (FileMaker's window only);
#   Xml     : with -Xml, Tools › Save a Copy as XML of the copy.
# `BASECRAFT_LAYOUT_DEMO=<out.fmp12> cargo test -p basecraft layout_demo_file`
# builds the file it was written for from the Contacts starter template.

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

function Test-FmLayout([string]$file, [string]$Layouts, [string]$Xml) {
  $copy = [IO.Path]::ChangeExtension($file, $null).TrimEnd('.') + "-check.fmp12"
  Copy-Item $file $copy -Force
  $open = Test-FmOpen $copy
  $out = [ordered]@{ Copy = $copy; Open = $open; Shots = @(); Xml = $null }
  if ($open -ne "OPEN") { return [pscustomobject]$out }
  Start-Fm $copy
  $i = 0
  foreach ($entry in $Layouts.Split(',')) {
    $name, $n = $entry.Split(':')
    # The layout pop-up of the status toolbar: top-level entries, 20 px apart.
    Click-Fm 130 123; Start-Sleep 1
    Click-Fm 128 (145 + 20 * ([int]$n - 1)); Start-Sleep 2
    $tag = ($name -replace '\W', '_')
    if ($i -eq 0) { Click-Fm 471 80; Start-Sleep 2 }   # a record to show
    $out.Shots += Save-FmMainShot "$tag-browse"
    Click-Fm 995 123; Start-Sleep 3                    # Edit Layout
    $out.Shots += Save-FmMainShot "$tag-layout"
    Click-Fm 1268 123; Close-FmLayoutQuestion          # Exit Layout
    $i++
  }
  if ($Xml) { Export-FmXml $Xml; $out.Xml = $Xml }
  Close-Fm
  [pscustomobject]$out
}
