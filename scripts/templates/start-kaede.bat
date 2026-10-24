@echo off
chcp 65001 >nul
cd /d "%~dp0"
title KaedeNiwa 内测
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-kaede.ps1"
echo.
echo 已退出。按任意键关闭窗口...
pause >nul
