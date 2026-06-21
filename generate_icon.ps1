Add-Type -AssemblyName System.Drawing

$size = 32
$bmp = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($bmp)

$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::ClearTypeGridFit

$bg = [System.Drawing.Color]::FromArgb(255, 30, 39, 54)
$g.Clear($bg)

$corner = 6
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$path.AddArc(0, 0, $corner*2, $corner*2, 180, 90)
$path.AddArc($size - $corner*2, 0, $corner*2, $corner*2, 270, 90)
$path.AddArc($size - $corner*2, $size - $corner*2, $corner*2, $corner*2, 0, 90)
$path.AddArc(0, $size - $corner*2, $corner*2, $corner*2, 90, 90)
$path.CloseFigure()

$region = New-Object System.Drawing.Region($path)
$g.SetClip($region, [System.Drawing.Drawing2D.CombineMode]::Replace)
$g.Clear($bg)
$g.ResetClip()

$yellow = [System.Drawing.Color]::FromArgb(255, 245, 197, 24)
$brush = New-Object System.Drawing.SolidBrush($yellow)
$font = New-Object System.Drawing.Font("Courier New", 11, [System.Drawing.FontStyle]::Bold)

$format = New-Object System.Drawing.StringFormat
$format.Alignment = [System.Drawing.StringAlignment]::Center
$format.LineAlignment = [System.Drawing.StringAlignment]::Center

$rect = New-Object System.Drawing.RectangleF(0, 0, $size, $size)
$g.DrawString(">_", $font, $brush, $rect, $format)

$g.Dispose()

$pngStream = New-Object System.IO.MemoryStream
$bmp.Save($pngStream, [System.Drawing.Imaging.ImageFormat]::Png)
$pngBytes = $pngStream.ToArray()
$pngStream.Dispose()
$bmp.Dispose()

$outPath = Join-Path $PSScriptRoot "nex_icon.ico"
$fs = [System.IO.File]::OpenWrite($outPath)
$writer = New-Object System.IO.BinaryWriter($fs)

# ICONDIR header
$writer.Write([uint16]0)   # reserved
$writer.Write([uint16]1)   # type: 1 = icon
$writer.Write([uint16]1)   # count: 1 image

# ICONDIRENTRY
$writer.Write([byte]$size)   # width
$writer.Write([byte]$size)   # height
$writer.Write([byte]0)       # color count (0 = no palette)
$writer.Write([byte]0)       # reserved
$writer.Write([uint16]1)     # planes
$writer.Write([uint16]32)    # bit count
$writer.Write([uint32]$pngBytes.Length)  # size of image data
$writer.Write([uint32]22)    # offset to image data (6 + 16)

# PNG image data
$writer.Write($pngBytes)
$writer.Close()
$fs.Close()

Write-Host "Icon saved to: $outPath"
