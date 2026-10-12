<#
.SYNOPSIS
  Build Square Input, refresh its data, and install the 64-bit and 32-bit TSF DLLs.

.DESCRIPTION
  1. Builds the DLL for x64 and x86 and refreshes the data directory
     (%APPDATA%\squareinput). Where local\ holds the harvested tables they are
     rebuilt with squareinput-data, and the results (codes, punct, homophones
     and the auto-promotion log) are copied to tsv\ to carry to another
     machine. Elsewhere the tables come from tsv\; its promotions.tsv only
     seeds a new data directory, so auto-promotions learnt here stay.
  2. Puts each DLL at its registered path (dist\squareinput.dll and
     dist\squareinput32.dll on a first install). Applications that loaded the old
     DLL keep it locked, but a loaded DLL can still be renamed, so the old file
     is moved aside and those applications keep running it until they restart.
  3. Registers a DLL with regsvr32 (the 32-bit one with SysWOW64\regsvr32)
     only on its first install or with -Register, which is needed when the
     registration itself changes, e.g. TSF categories. TSF profiles live in
     HKLM, so that step raises a UAC prompt; plain code updates need none.
  4. Adds "Square Input" to the keyboards of the Chinese (Hong Kong) language, unless
     -NoLanguage is given. Switch to it with Win+Space.

  Moved-aside DLLs that nothing has loaded any more are deleted.
#>
param([switch]$Register, [switch]$NoLanguage)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$clsid = '{89B0ACE9-CFE0-4081-A7C3-7FBFB6082420}'
$tip = "0404:$clsid{8D43BCFA-667D-4A00-9383-968A0E98609E}"
$dist = Join-Path $root 'dist'

function Invoke-Cargo([string[]]$CargoArgs) {
    cargo @CargoArgs
    if ($LASTEXITCODE) { throw "cargo $CargoArgs failed" }
}

$data = if ($env:SQUAREINPUT_DATA) { $env:SQUAREINPUT_DATA } else { Join-Path $env:APPDATA 'squareinput' }
$tsv = Join-Path $root 'tsv'
$tables = 'codes.tsv', 'punct.tsv', 'homophones.tsv'
if ((Test-Path "$root\local\codes.tsv") -and (Test-Path "$root\local\punct.tsv")) {
    Invoke-Cargo 'build', '--manifest-path', "$root\Cargo.toml", '--release', '-p', 'squareinput'
    & "$root\target\release\squareinput-data.exe"
    if ($LASTEXITCODE) { throw 'squareinput-data failed' }
    New-Item -ItemType Directory -Force $tsv | Out-Null
    foreach ($name in $tables + 'promotions.tsv') {
        if (Test-Path "$data\$name") { Copy-Item "$data\$name" $tsv -Force }
    }
} elseif ((Test-Path "$tsv\codes.tsv") -and (Test-Path "$tsv\punct.tsv")) {
    New-Item -ItemType Directory -Force $data | Out-Null
    foreach ($name in $tables) {
        if (Test-Path "$tsv\$name") { Copy-Item "$tsv\$name" $data -Force }
    }
    if ((Test-Path "$tsv\promotions.tsv") -and -not (Test-Path "$data\promotions.tsv")) {
        Copy-Item "$tsv\promotions.tsv" $data
    }
    Write-Host "copied the tables from tsv into $data"
} elseif (-not (Test-Path "$data\codes.tsv")) {
    Write-Warning "no tables in local\ or tsv\: put codes.tsv, punct.tsv, homophones.tsv and promotions.tsv in tsv\"
}

New-Item -ItemType Directory -Force $dist | Out-Null
$installed = @()

foreach ($arch in @(
        @{ Target = 'x86_64-pc-windows-msvc'; Classes = 'CLSID'; Name = 'squareinput.dll'; Regsvr32 = "$env:WINDIR\System32\regsvr32.exe" },
        @{ Target = 'i686-pc-windows-msvc'; Classes = 'WOW6432Node\CLSID'; Name = 'squareinput32.dll'; Regsvr32 = "$env:WINDIR\SysWOW64\regsvr32.exe" })) {
    Invoke-Cargo 'build', '--manifest-path', "$root\Cargo.toml", '--release', '-p', 'squareinput-tsf', '--target', $arch.Target
    $built = "$root\target\$($arch.Target)\release\squareinput_tsf.dll"

    $key = "Registry::HKEY_CLASSES_ROOT\$($arch.Classes)\$clsid\InprocServer32"
    $registered = (Get-ItemProperty $key -ErrorAction SilentlyContinue).'(default)'
    $target = if ($registered) { $registered } else { Join-Path $dist $arch.Name }

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
    $installed += $target

    if ($Register -or -not $registered) {
        $reg = Start-Process -FilePath $arch.Regsvr32 -ArgumentList '/s', "`"$target`"" -Verb RunAs -Wait -PassThru
        if ($reg.ExitCode) { throw "$($arch.Regsvr32) failed with exit code $($reg.ExitCode)" }
        Write-Host "registered $target"
    }
}

if (-not $NoLanguage) {
    $languages = Get-WinUserLanguageList
    $hk = $languages | Where-Object LanguageTag -like 'zh-Hant-HK' | Select-Object -First 1
    if (-not $hk) { $hk = $languages | Where-Object LanguageTag -like 'zh-*' | Select-Object -First 1 }
    if (-not $hk) { throw 'no Chinese language in the language list; add Chinese (Hong Kong) first' }
    if ($hk.InputMethodTips -notcontains $tip) {
        $hk.InputMethodTips.Add($tip)
        Set-WinUserLanguageList $languages -Force
        Write-Host "added Square Input to $($hk.LanguageTag)"
    }
}

# Delete moved-aside and superseded DLLs that are no longer loaded anywhere.
Get-ChildItem $dist -Filter 'squareinput*' | Where-Object FullName -notin $installed | ForEach-Object {
    try { Remove-Item $_.FullName -ErrorAction Stop } catch { }
}
