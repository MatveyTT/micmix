# Рисует исходную иконку приложения app-icon.png (1024×1024).
# Потом: npx tauri icon app-icon.png -o src-tauri/icons
Add-Type -AssemblyName System.Drawing
$S = 1024
$bmp = New-Object Drawing.Bitmap $S, $S
$g = [Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'
$g.Clear([Drawing.Color]::Transparent)

function RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object Drawing.Drawing2D.GraphicsPath
    $p.AddArc($x, $y, 2 * $r, 2 * $r, 180, 90)
    $p.AddArc($x + $w - 2 * $r, $y, 2 * $r, 2 * $r, 270, 90)
    $p.AddArc($x + $w - 2 * $r, $y + $h - 2 * $r, 2 * $r, 2 * $r, 0, 90)
    $p.AddArc($x, $y + $h - 2 * $r, 2 * $r, 2 * $r, 90, 90)
    $p.CloseFigure()
    return $p
}

# Фон: скруглённый квадрат с градиентом фиолетовый → бирюзовый.
$bg = RoundRect 40 40 944 944 220
$grad = New-Object Drawing.Drawing2D.LinearGradientBrush(
    (New-Object Drawing.Point 0, 0), (New-Object Drawing.Point $S, $S),
    [Drawing.Color]::FromArgb(255, 124, 58, 237), [Drawing.Color]::FromArgb(255, 6, 182, 212))
$g.FillPath($grad, $bg)

$white = New-Object Drawing.SolidBrush ([Drawing.Color]::White)
$pen = New-Object Drawing.Pen ([Drawing.Color]::White), 44
$pen.StartCap = 'Round'; $pen.EndCap = 'Round'

# Капсула микрофона.
$g.FillPath($white, (RoundRect 382 190 200 380 100))
# Дужка держателя, ножка и подставка.
$g.DrawArc($pen, 302, 330, 360, 330, 0, 180)
$g.DrawLine($pen, 482, 662, 482, 770)
$g.DrawLine($pen, 392, 790, 572, 790)

# Звуковые волны справа — «звук идёт в микрофон».
$wave = New-Object Drawing.Pen ([Drawing.Color]::FromArgb(230, 255, 255, 255)), 34
$wave.StartCap = 'Round'; $wave.EndCap = 'Round'
$g.DrawArc($wave, 640, 300, 120, 240, -60, 120)
$g.DrawArc($wave, 680, 240, 200, 360, -55, 110)

$out = Join-Path $PSScriptRoot 'app-icon.png'
$bmp.Save($out, [Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Host "Иконка: $out"
