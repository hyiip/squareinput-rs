<#
.SYNOPSIS
  Build Square Input, refresh its data, and register the TSF DLL.

.DESCRIPTION
  1. Builds the release DLL and rebuilds %APPDATA%\squareinput from the harvested tables.
  2. Copies the DLL to dist\squareinput-<timestamp>.dll. Every running application
     that has loaded an older copy keeps that file locked, so each install gets
     a new name instead of overwriting.
  3. Registers it with regsvr32. TSF profiles live in HKLM, so this step
     raises a UAC prompt.
  4. Adds "Square Input" to the keyboards of the 中文(香港) language, unless
     -NoLanguage is given. Switch to it with Win+Space.

  Older dist\squareinput-*.dll files that nothing has loaded any more are deleted.
#>
param([switch]$NoLanguage)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$tip = '0404:{89B0ACE9-CFE0-4081-A7C3-7FBFB6082420}{8D43BCFA-667D-4A00-9383-968A0E98609E}'

cargo build --manifest-path "$root\Cargo.toml" --release -p squareinput-tsf -p squareinput
if ($LASTEXITCODE) { throw 'cargo build failed' }
& "$root\target\release\squareinput-data.exe"
if ($LASTEXITCODE) { throw 'squareinput-data failed' }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$dll = Join-Path $dist ("squareinput-{0}.dll" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
Copy-Item "$root\target\release\squareinput_tsf.dll" $dll

$reg = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList '/s', "`"$dll`"" -Verb RunAs -Wait -PassThru
if ($reg.ExitCode) { throw "regsvr32 failed with exit code $($reg.ExitCode)" }
Write-Host "registered $dll"

if (-not $NoLanguage) {
    $languages = Get-WinUserLanguageList
    $hk = $languages | Where-Object LanguageTag -like 'zh-Hant-HK' | Select-Object -First 1
    if (-not $hk) { $hk = $languages | Where-Object LanguageTag -like 'zh-*' | Select-Object -First 1 }
    if (-not $hk) { throw 'no Chinese language in the language list; add 中文(香港) first' }
    if ($hk.InputMethodTips -notcontains $tip) {
        $hk.InputMethodTips.Add($tip)
        Set-WinUserLanguageList $languages -Force
        Write-Host "added Square Input to $($hk.LanguageTag)"
    }
}

# Remove older copies that are no longer loaded anywhere (locked files fail to delete).
Get-ChildItem $dist -Filter 'squareinput-*.dll' | Where-Object FullName -ne $dll | ForEach-Object {
    try { Remove-Item $_.FullName -ErrorAction Stop } catch { }
}
