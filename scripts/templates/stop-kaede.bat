@echo off
chcp 65001 >nul
echo 正在停止 KaedeNiwa 所有进程...
taskkill /F /IM kaede-server.exe 2>nul
taskkill /F /IM livekit-server.exe 2>nul
taskkill /F /IM kaedeniwa.exe 2>nul
echo 完成。
timeout /t 2 >nul
