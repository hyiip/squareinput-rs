<#
.SYNOPSIS
  Build Square Input, refresh its data, and install the TSF DLL.

.DESCRIPTION
  1. Builds the release DLL and rebuilds %APPDATA%\squareinput from the harvested tables.
  2. Puts the new DLL at the registered path (dist\squareinput.dll on a first
     install). Applications that loaded the old DLL keep it locked, but a
     loaded DLL can still be renamed, so the old file is moved aside and
     those applications keep running it until they restart.
  3. Registers it with regsvr32 only on a first install or with -Register
     (needed when the registration itself changes, e.g. TSF categories).
     TSF profiles live in HKLM, so that step raises a UAC prompt; plain code
     updates need none.
  4. Adds "Square Input" to the keyboards of the 中文(香港) language, unless
     -NoLanguage is given. Switch to it with Win+Space.

  Moved-aside DLLs that nothing has loaded any more are deleted.
#>
param([switch]$Register, [switch]$NoLanguage)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$clsid = '{89B0ACE9-CFE0-4081-A7C3-7FBFB6082420}'
$tip = "0404:$clsid{8D43BCFA-667D-4A00-9383-968A0E98609E}"

cargo build --manifest-path "$root\Cargo.toml" --release -p squareinput-tsf -p squareinput
if ($LASTEXITCODE) { throw 'cargo build failed' }
& "$root\target\release\squareinput-data.exe"
if ($LASTEXITCODE) { throw 'squareinput-data failed' }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$registered = (Get-ItemProperty "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid\InprocServer32" -ErrorAction SilentlyContinue).'(default)'
$target = if ($registered) { $registered } else { Join-Path $dist 'squareinput.dll' }
$built = "$root\target\release\squareinput_tsf.dll"

if (Test-Path $target) {
    try {
        Copy-Item $built $target -Force -ErrorAction Stop
    } catch {
        $aside = '{0}.old-{1}' -f $target, (Get-Date -Format 'yyyyMMdd-HHmmss')
        Move-Item $target $aside
        Copy-Item $built $target
        Write-Host "old DLL is in use; moved it to $aside"
    }
} else {
    Copy-Item $built $target
}
Write-Host "installed $target"

if ($Register -or -not $registered) {
    $reg = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList '/s', "`"$target`"" -Verb RunAs -Wait -PassThru
    if ($reg.ExitCode) { throw "regsvr32 failed with exit code $($reg.ExitCode)" }
    Write-Host "registered $target"
}

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

# Delete moved-aside and superseded DLLs that are no longer loaded anywhere.
Get-ChildItem $dist -Filter 'squareinput*' | Where-Object FullName -ne $target | ForEach-Object {
    try { Remove-Item $_.FullName -ErrorAction Stop } catch { }
}
