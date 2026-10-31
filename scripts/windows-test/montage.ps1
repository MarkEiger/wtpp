# Combine screenshots into 2x2 contact sheets (sheet-N.png) for review.
# -Clean deletes the previous run's screenshots instead.
param([string]$Dir = "$env:TEMP\wtpp-test", [switch]$Clean)
if ($Clean) { Get-ChildItem $Dir -Filter '*.png' | Remove-Item; return }
Add-Type -AssemblyName System.Drawing
Get-ChildItem $Dir -Filter 'sheet-*.png' | Remove-Item
$files = Get-ChildItem $Dir -Filter '*-*.png' | Where-Object { $_.Name -notlike 'sheet-*' } | Sort-Object LastWriteTime
$w = 760; $h = 500; $n = 0
for ($i = 0; $i -lt $files.Count; $i += 4) {
  $sheet = New-Object Drawing.Bitmap (2 * $w), (2 * $h + 40)
  $g = [Drawing.Graphics]::FromImage($sheet); $g.Clear([Drawing.Color]::White)
  $g.InterpolationMode = 'HighQualityBicubic'
  $font = New-Object Drawing.Font 'Segoe UI', 11
  for ($k = 0; $k -lt 4 -and $i + $k -lt $files.Count; $k++) {
    $img = [Drawing.Image]::FromFile($files[$i + $k].FullName)
    $x = ($k % 2) * $w; $y = [math]::Floor($k / 2) * ($h + 20)
    $scale = [math]::Min($w / $img.Width, $h / $img.Height)
    $g.DrawImage($img, $x, $y + 20, $img.Width * $scale, $img.Height * $scale)
    $g.DrawString($files[$i + $k].BaseName, $font, [Drawing.Brushes]::Black, $x + 4, $y + 1)
    $img.Dispose()
  }
  $n++; $sheet.Save("$Dir\sheet-$n.png"); $g.Dispose(); $sheet.Dispose()
}
"$n sheets"
