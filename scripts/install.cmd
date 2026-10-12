@echo off
rem Runs install.ps1 whatever the PowerShell execution policy, which on a new
rem machine, or for a script unpacked from a download, refuses to run it.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
pause
