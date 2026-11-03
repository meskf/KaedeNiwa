@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-kaede.ps1"
if errorlevel 1 pause
