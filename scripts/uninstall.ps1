<#
.SYNOPSIS
  Remove Square Input from the keyboard list and unregister the TSF DLL.

.DESCRIPTION
  Leaves %APPDATA%\squareinput (tables and the auto-promotion log) in place. Applications
  that still have the DLL loaded keep it until they restart.
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$tip = '0404:{89B0ACE9-CFE0-4081-A7C3-7FBFB6082420}{8D43BCFA-667D-4A00-9383-968A0E98609E}'

$languages = Get-WinUserLanguageList
foreach ($l in $languages) { [void]$l.InputMethodTips.Remove($tip) }
Set-WinUserLanguageList $languages -Force

$dll = Get-ChildItem (Join-Path $root 'dist') -Filter 'squareinput-*.dll' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime | Select-Object -Last 1
if ($dll) {
    $reg = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList '/s', '/u', "`"$($dll.FullName)`"" -Verb RunAs -Wait -PassThru
    if ($reg.ExitCode) { throw "regsvr32 /u failed with exit code $($reg.ExitCode)" }
    Write-Host "unregistered $($dll.FullName)"
}
