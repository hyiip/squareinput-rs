<#
.SYNOPSIS
  Remove Square Input from the keyboard list and unregister the 64-bit and 32-bit TSF DLLs.

.DESCRIPTION
  Finds the DLLs at their registered paths, so it works for an install from
  source and from a release alike. Leaves %APPDATA%\squareinput (tables and
  the auto-promotion log) and the DLL files in place. Applications that still
  have the DLL loaded keep it until they restart.
#>
$ErrorActionPreference = 'Stop'
$clsid = '{89B0ACE9-CFE0-4081-A7C3-7FBFB6082420}'
$tip = "0404:$clsid{8D43BCFA-667D-4A00-9383-968A0E98609E}"

$languages = Get-WinUserLanguageList
foreach ($l in $languages) { [void]$l.InputMethodTips.Remove($tip) }
Set-WinUserLanguageList $languages -Force

foreach ($arch in @(
        @{ Classes = 'CLSID'; Regsvr32 = "$env:WINDIR\System32\regsvr32.exe" },
        @{ Classes = 'WOW6432Node\CLSID'; Regsvr32 = "$env:WINDIR\SysWOW64\regsvr32.exe" })) {
    $key = "Registry::HKEY_CLASSES_ROOT\$($arch.Classes)\$clsid\InprocServer32"
    $dll = (Get-ItemProperty $key -ErrorAction SilentlyContinue).'(default)'
    if (-not $dll) { continue }
    $reg = Start-Process -FilePath $arch.Regsvr32 -ArgumentList '/s', '/u', "`"$dll`"" -Verb RunAs -Wait -PassThru
    # Keep going, so a failure in one leaves the other unregistered too.
    if ($reg.ExitCode) { Write-Warning "$($arch.Regsvr32) /u failed with exit code $($reg.ExitCode)" }
    else { Write-Host "unregistered $dll" }
}
