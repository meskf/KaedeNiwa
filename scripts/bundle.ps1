# KaedeNiwa 一键全量打包脚本
# 用法: .\scripts\bundle.ps1 [web|desktop|server]
param(
    [Parameter(Mandatory=$true)]
    [ValidateSet("web", "desktop", "server")]
    [string]$Platform
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

if ($Platform -eq 'web' -or $Platform -eq 'desktop') {
    if (-not (Get-Command dx -ErrorAction SilentlyContinue)) {
        Write-Host "错误: 未找到 dx 命令。请先 cargo install dioxus-cli" -ForegroundColor Red
        exit 1
    }
}

switch ($Platform) {
    "web" {
        Write-Host "打包 Web (release-wasm)..." -ForegroundColor Cyan
        Push-Location crates\kaede-wasm
        dx bundle --platform web --profile release-wasm --out-dir ..\..\dist\web
        Pop-Location
        node scripts\postbuild.cjs dist\web\public
        Write-Host "`n输出: dist\web\public" -ForegroundColor Green
    }
    "desktop" {
        Write-Host "打包 Desktop (Wry)..." -ForegroundColor Cyan
        Push-Location crates\kaede-desktop
        dx bundle --platform desktop --release --out-dir ..\..\dist\desktop
        Pop-Location
        
        $exe = "dist\desktop\kaedeniwa.exe"
        if (-not (Test-Path $exe)) {
            $exe = (Get-ChildItem -Path "dist\desktop" -Filter "*.exe" | Select-Object -First 1).FullName
        }
        $outDir = "dist\desktop"
        
        Write-Host "收集 GStreamer 依赖 (这可能需要几分钟)..." -ForegroundColor Yellow
        $gstRoot = $env:GSTREAMER_1_0_ROOT_MSVC_X86_64
        if (-not $gstRoot) { $gstRoot = "C:\Program Files\gstreamer\1.0\msvc_x86_64" }
        
        if (Test-Path $gstRoot) {
            # 1. 拷贝核心 DLL
            $binDlls = Get-ChildItem "$gstRoot\bin\*.dll"
            foreach ($d in $binDlls) { Copy-Item $d.FullName -Destination $outDir -Force }
            
            # 2. 拷贝插件
            $pluginDir = Join-Path $outDir "lib\gstreamer-1.0"
            New-Item -ItemType Directory -Force $pluginDir | Out-Null
            $plugins = Get-ChildItem "$gstRoot\lib\gstreamer-1.0\*.dll"
            foreach ($p in $plugins) { Copy-Item $p.FullName -Destination $pluginDir -Force }
            Write-Host "  [OK] GStreamer 已打包至: $outDir" -ForegroundColor Green
        } else {
            Write-Host "  [WARN] 未找到 GStreamer 安装目录，客户端可能无法启动媒体功能。" -ForegroundColor Red
        }
        
        Write-Host "`n输出: $outDir (解压即用)" -ForegroundColor Green
    }
    "server" {
        Write-Host "打包 Server..." -ForegroundColor Cyan
        cargo build --release -p kaede-server
        
        $outDir = "dist\server"
        New-Item -ItemType Directory -Force $outDir | Out-Null
        
        # 1. 拷贝主程序
        Copy-Item "target\release\kaede-server.exe" -Destination $outDir -Force
        
        # 2. 拷贝 LiveKit
        $lkExe = "tools\livekit\livekit-server.exe"
        if (Test-Path $lkExe) {
            Copy-Item $lkExe -Destination $outDir -Force
        } else {
            Write-Host "  [WARN] 未找到 tools\livekit\livekit-server.exe，将无法自动开启语音功能！" -ForegroundColor Red
        }
        
        # 3. 准备配置文件（.env.example 和默认 .env）
        Copy-Item "deploy\.env.example" -Destination "$outDir\.env.example" -Force
        if (-not (Test-Path "$outDir\.env")) {
            # 生成一个默认的开发用 .env
            $envContent = @"
# 本地开箱即用配置：默认只监听回环地址，避免把开发实例暴露到局域网。
# 需要局域网访问时改成 KAEDE_BIND=0.0.0.0:18080，并自行更换 LiveKit 密钥。
KAEDE_BIND=127.0.0.1:18080
KAEDE_DB=sqlite://kaede.db?mode=rwc
KAEDE_ADMIN_ENABLED=0
LIVEKIT_API_KEY=devkey
LIVEKIT_API_SECRET=secret
LIVEKIT_WS_URL=ws://127.0.0.1:18082
"@
            [System.IO.File]::WriteAllText((Join-Path (Get-Location) "$outDir\.env"), $envContent, [System.Text.UTF8Encoding]::new($false))
        }
        
        # 4. 附带一个启动脚本，自动加载 .env
        $runBat = @"
@echo off
chcp 65001 > nul
title KaedeNiwa Server
echo 正在启动 KaedeNiwa 服务端 (含 LiveKit)...
rem 加载同目录 .env（服务端启动会强制校验 LIVEKIT_API_KEY/SECRET）。
for /f "usebackq eol=# tokens=1,* delims==" %%A in (".env") do set "%%A=%%B"
set KAEDE_LOG_FORMAT=human
kaede-server.exe
pause
"@
        [System.IO.File]::WriteAllText((Join-Path (Get-Location) "$outDir\start.bat"), $runBat, [System.Text.Encoding]::Default)
        
        Write-Host "`n输出: $outDir (双击 start.bat 即可)" -ForegroundColor Green
    }
}