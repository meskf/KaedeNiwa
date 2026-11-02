# KaedeNiwa 一键打包脚本（Vite + wasm-pack + Electron + embed-web）
# 用法: .\scripts\bundle.ps1 [web|server|desktop|all]
param(
    [Parameter(Mandatory=$true)]
    [ValidateSet("web", "server", "desktop", "all")]
    [string]$Platform
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root

function Build-Web {
    Write-Host "==> 构建 Web (wasm-pack + vite)" -ForegroundColor Cyan
    Push-Location apps\web
    try {
        npm run build:wasm
        if ($LASTEXITCODE -ne 0) { throw "build:wasm 失败" }
        npm run build
        if ($LASTEXITCODE -ne 0) { throw "vite build 失败" }
    } finally { Pop-Location }
    if (-not (Test-Path apps\web\dist\index.html)) { throw "apps/web/dist 缺少 index.html" }
    Write-Host "Web 产物: $root\apps\web\dist" -ForegroundColor Green
}

function Build-Server {
    Write-Host "==> 构建 Server (embed-web 单二进制)" -ForegroundColor Cyan
    if (-not (Test-Path apps\web\dist\index.html)) {
        throw "apps/web/dist 不存在。请先运行: .\scripts\bundle.ps1 web"
    }
    cargo build --release -p kaede-server --features embed-web
    if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }

    $outDir = "dist\server"
    New-Item -ItemType Directory -Force $outDir | Out-Null
    Copy-Item "target\release\kaede-server.exe" -Destination $outDir -Force

    $lkExe = "tools\livekit\livekit-server.exe"
    if (Test-Path $lkExe) {
        Copy-Item $lkExe -Destination $outDir -Force
    } else {
        Write-Host "  [WARN] 未找到 tools\livekit\livekit-server.exe，语音需自行配置 LiveKit" -ForegroundColor Yellow
    }

    Copy-Item "deploy\.env.example" -Destination "$outDir\.env.example" -Force
    if (-not (Test-Path "$outDir\.env")) {
        $envContent = @"
# 本地开箱即用配置：默认只监听回环地址。
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

    $runBat = @"
@echo off
chcp 65001 > nul
title KaedeNiwa Server
echo 正在启动 KaedeNiwa 服务端 (含 LiveKit)...
for /f "usebackq eol=# tokens=1,* delims==" %%A in (".env") do set "%%A=%%B"
set KAEDE_LOG_FORMAT=human
kaede-server.exe
pause
"@
    [System.IO.File]::WriteAllText((Join-Path (Get-Location) "$outDir\start.bat"), $runBat, [System.Text.Encoding]::Default)

    Write-Host "Server 产物: $outDir (双击 start.bat 即可)" -ForegroundColor Green
}

function Build-Desktop {
    Write-Host "==> 构建 Desktop (Electron)" -ForegroundColor Cyan
    Push-Location apps\desktop
    try {
        if (-not (Test-Path node_modules)) { npm ci }
        npm run build:client
        if ($LASTEXITCODE -ne 0) { throw "electron-builder 失败" }
    } finally { Pop-Location }
    $unpacked = "release\desktop\win-unpacked"
    if (Test-Path $unpacked) {
        $exe = Get-ChildItem $unpacked -Filter *.exe | Select-Object -First 1
        Write-Host "Desktop 产物: $unpacked\$($exe.Name)" -ForegroundColor Green
    } else {
        Write-Host "Desktop 产物: release\desktop" -ForegroundColor Yellow
    }
}

switch ($Platform) {
    "web"     { Build-Web }
    "server"  { Build-Server }
    "desktop" { Build-Desktop }
    "all"     { Build-Web; Build-Server; Build-Desktop }
}

