@echo off
rem Runs install-apo.ps1 as administrator.
rem Examples: install.bat, install.bat -Status, install.bat -Uninstall, install.bat -Device HyperX
powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process powershell -Verb RunAs -ArgumentList ('-NoProfile -ExecutionPolicy Bypass -NoExit -File \"%~dp0install-apo.ps1\" %*')"
