@echo off
rem Runs uninstall.ps1 whatever the PowerShell execution policy.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" %*
pause
