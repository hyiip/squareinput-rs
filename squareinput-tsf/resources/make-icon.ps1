<#
.SYNOPSIS
  Draw the icons embedded in the Windows DLL: squareinput.ico and english.ico.

.DESCRIPTION
  squareinput.ico is the 3x3 grid of the macOS menu icon, black on a white
  tile: on a light taskbar it reads as the black grid alone, and on a dark
  one the tile keeps it visible. english.ico is the same tile with 英, shown
  on the taskbar while Shift has switched to English. Each size is drawn on
  whole pixels, so the small ones stay sharp. Sizes below 256 are stored as
  32-bit bitmaps, 256 as PNG.
#>
param([string]$Dir = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$tile = [Drawing.Color]::White
$sizes = 16, 20, 24, 32, 40, 48, 64, 256

# The tile: a rounded square with a small margin. Calls $content with the
# graphics, the size, the margin and the side.
function New-IconBitmap([int]$s, [scriptblock]$content) {
    $bmp = [Drawing.Bitmap]::new($s, $s, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.Clear([Drawing.Color]::Transparent)

    $m = [int][Math]::Round($s / 32)
    $side = $s - 2 * $m
    $d = [Math]::Max(4, $side / 3)
    $path = [Drawing.Drawing2D.GraphicsPath]::new()
    $path.AddArc($m, $m, $d, $d, 180, 90)
    $path.AddArc($m + $side - $d, $m, $d, $d, 270, 90)
    $path.AddArc($m + $side - $d, $m + $side - $d, $d, $d, 0, 90)
    $path.AddArc($m, $m + $side - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    $g.SmoothingMode = 'AntiAlias'
    $g.FillPath([Drawing.SolidBrush]::new($tile), $path)

    & $content $g $s $m $side
    $g.Dispose()
    $bmp
}

# The grid: four lines each way, $w pixels wide and $k apart, centred.
$grid = {
    param($g, $s, $m, $side)
    $w = [Math]::Max(1, [int][Math]::Floor($s / 16))
    $k = [int][Math]::Floor(($side * 0.75 - $w) / 3)
    $span = 3 * $k + $w
    $start = $m + [int][Math]::Floor(($side - $span) / 2)
    $g.SmoothingMode = 'None'
    for ($i = 0; $i -le 3; $i++) {
        $p = $start + $i * $k
        $g.FillRectangle([Drawing.Brushes]::Black, $p, $start, $w, $span)
        $g.FillRectangle([Drawing.Brushes]::Black, $start, $p, $span, $w)
    }
}

# 英 drawn $em pixels high with the font's hinting, on a scratch bitmap.
# Returns the bitmap and the box its ink covers.
function Get-Glyph([int]$em) {
    $bmp = [Drawing.Bitmap]::new(2 * $em, 2 * $em, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.Clear([Drawing.Color]::Transparent)
    $g.TextRenderingHint = 'AntiAliasGridFit'
    $font = [Drawing.Font]::new('Microsoft JhengHei UI', $em, [Drawing.FontStyle]::Bold, [Drawing.GraphicsUnit]::Pixel)
    $g.DrawString([string][char]0x82F1, $font, [Drawing.Brushes]::Black, 0, 0)
    $font.Dispose()
    $g.Dispose()
    $left = $top = [int]::MaxValue
    $right = $bottom = -1
    for ($y = 0; $y -lt $bmp.Height; $y++) {
        for ($x = 0; $x -lt $bmp.Width; $x++) {
            if ($bmp.GetPixel($x, $y).A -gt 0) {
                $left = [Math]::Min($left, $x); $right = [Math]::Max($right, $x)
                $top = [Math]::Min($top, $y); $bottom = [Math]::Max($bottom, $y)
            }
        }
    }
    @{ Bitmap = $bmp; Ink = [Drawing.Rectangle]::new($left, $top, $right - $left + 1, $bottom - $top + 1) }
}

# 英 as large as fits in 80% of the tile, centred on whole pixels so the
# hinting stays sharp.
$english = {
    param($g, $s, $m, $side)
    $span = [int][Math]::Floor($side * 0.8)
    for ($em = $side; $em -gt 6; $em--) {
        $glyph = Get-Glyph $em
        if ($glyph.Ink.Width -le $span -and $glyph.Ink.Height -le $span) { break }
        $glyph.Bitmap.Dispose()
    }
    $ink = $glyph.Ink
    $x = $m + [int][Math]::Floor(($side - $ink.Width) / 2)
    $y = $m + [int][Math]::Floor(($side - $ink.Height) / 2)
    $g.DrawImage($glyph.Bitmap, [Drawing.Rectangle]::new($x, $y, $ink.Width, $ink.Height), $ink, 'Pixel')
    $glyph.Bitmap.Dispose()
}

# An icon image as a DIB: BITMAPINFOHEADER (height doubled for the mask),
# BGRA rows bottom-up, then an all-clear AND mask (alpha does the masking).
function Get-DibBytes([Drawing.Bitmap]$bmp) {
    $s = $bmp.Width
    $data = $bmp.LockBits([Drawing.Rectangle]::new(0, 0, $s, $s), 'ReadOnly', 'Format32bppArgb')
    $pixels = [byte[]]::new($s * $s * 4)
    [Runtime.InteropServices.Marshal]::Copy($data.Scan0, $pixels, 0, $pixels.Length)
    $bmp.UnlockBits($data)
    $maskRow = [int][Math]::Ceiling($s / 32) * 4
    $ms = [IO.MemoryStream]::new()
    $bw = [IO.BinaryWriter]::new($ms)
    $bw.Write([int]40); $bw.Write([int]$s); $bw.Write([int]($s * 2))
    $bw.Write([int16]1); $bw.Write([int16]32); $bw.Write([int]0)
    $bw.Write([int]($pixels.Length + $maskRow * $s))
    $bw.Write([int]0); $bw.Write([int]0); $bw.Write([int]0); $bw.Write([int]0)
    for ($y = $s - 1; $y -ge 0; $y--) { $bw.Write($pixels, $y * $s * 4, $s * 4) }
    $bw.Write([byte[]]::new($maskRow * $s))
    $bw.Flush()
    $ms.ToArray()
}

function Write-Icon([string]$out, [scriptblock]$content) {
    $images = foreach ($s in $sizes) {
        $bmp = New-IconBitmap $s $content
        if ($s -ge 256) {
            $ms = [IO.MemoryStream]::new()
            $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
            $bytes = $ms.ToArray()
        } else {
            $bytes = Get-DibBytes $bmp
        }
        $bmp.Dispose()
        , @($s, $bytes)
    }

    $fs = [IO.File]::Create($out)
    $bw = [IO.BinaryWriter]::new($fs)
    $bw.Write([int16]0); $bw.Write([int16]1); $bw.Write([int16]$images.Count)
    $offset = 6 + 16 * $images.Count
    foreach ($img in $images) {
        $s, $bytes = $img
        $dim = if ($s -ge 256) { 0 } else { $s }
        $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
        $bw.Write([int16]1); $bw.Write([int16]32); $bw.Write([int]$bytes.Length); $bw.Write([int]$offset)
        $offset += $bytes.Length
    }
    foreach ($img in $images) { $bw.Write([byte[]]$img[1]) }
    $bw.Dispose()
    "wrote $out ($((Get-Item $out).Length) bytes, sizes $($sizes -join ' '))"
}

Write-Icon (Join-Path $Dir 'squareinput.ico') $grid
Write-Icon (Join-Path $Dir 'english.ico') $english
