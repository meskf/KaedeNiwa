# KaedeNiwa 一键全栈启动

param(
    [switch]$SkipTest,
    [switch]$CleanDb
)

$ErrorActionPreference = 'Continue'
$root = "D:\dev2\kaedeniwa"

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  KaedeNiwa 一键启动" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

Remove-Item Env:KAEDE_SKIP_LIVEKIT_ENV_CHECK -ErrorAction SilentlyContinue

# ---- 1. 清理旧进程 ----
Write-Host "[1/6] 清理旧进程..." -ForegroundColor Yellow
@('node', 'kaede-server', 'livekit-server') | ForEach-Object {
    $procs = Get-Process $_ -ErrorAction SilentlyContinue
    if ($procs) {
        $procs | ForEach-Object {
            Write-Host "   kill $($_.Name) (PID $($_.Id))" -ForegroundColor DarkGray
            Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
        }
    }
}
@(18080, 7880, 7881, 7882, 5173, 5174, 5175) | ForEach-Object {
    Get-NetTCPConnection -LocalPort $_ -State Listen -ErrorAction SilentlyContinue | ForEach-Object {
        Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
    }
}
Start-Sleep -Seconds 2

# ---- 2. 清空数据库 ----
if ($CleanDb) {
    Write-Host "[2/6] 清空数据库..." -ForegroundColor Yellow
    Get-ChildItem "$root\kaede.db*" -ErrorAction SilentlyContinue | Remove-Item -Force
    Write-Host "   数据库已清空" -ForegroundColor Green
} else {
    Write-Host "[2/6] 保留数据库（如需清空加 -CleanDb）" -ForegroundColor DarkGray
}

# ---- 3. 启动 LiveKit ----
Write-Host "[3/6] 启动 LiveKit..." -ForegroundColor Yellow
$lkExe = "$root\tools\livekit\livekit-server.exe"
if (-not (Test-Path $lkExe)) {
    Write-Host "   ❌ 找不到 $lkExe" -ForegroundColor Red
    Read-Host "按回车退出"; exit 1
}
Start-Process -FilePath $lkExe -ArgumentList '--dev' -WindowStyle Normal

# ---- 4. 启动后端 ----
Write-Host "[4/6] 启动后端..." -ForegroundColor Yellow
$backendCmd = @"
Remove-Item Env:KAEDE_SKIP_LIVEKIT_ENV_CHECK -ErrorAction SilentlyContinue
`$env:LIVEKIT_API_KEY = 'devkey'
`$env:LIVEKIT_API_SECRET = 'secret'
`$env:LIVEKIT_WS_URL = 'ws://127.0.0.1:7880'
`$env:RUST_LOG = 'info,kaede_server=debug'
Set-Location '$root'
Write-Host '=== Kaede 后端 ===' -ForegroundColor Cyan
cargo run -p kaede-server --bin kaede-server --features dev-mode
"@
Start-Process pwsh -ArgumentList '-NoExit', '-Command', $backendCmd -WindowStyle Normal

# ---- 5. 启动前端 ----
# [4.5/6] 重建 wasm（客户端 Rust 改动必须重新编译进 wasm，否则 e2e 会跑旧代码）
Write-Host "[4.5/6] 重建 wasm..." -ForegroundColor Cyan
Push-Location (Join-Path $root "apps\web")
npm run build:wasm 2>&1 | Out-Null
$wasmExit = $LASTEXITCODE
Pop-Location
if ($wasmExit -ne 0) {
    Write-Host "wasm build failed (exit $wasmExit)" -ForegroundColor Red
    exit 1
}
Write-Host "  ✅ wasm 已重建" -ForegroundColor Green

Write-Host "[5/6] 启动前端..." -ForegroundColor Yellow
$frontendCmd = "Set-Location '$root\apps\web'; Write-Host '=== Vite 前端 ===' -ForegroundColor Cyan; npm run dev"
Start-Process pwsh -ArgumentList '-NoExit', '-Command', $frontendCmd -WindowStyle Normal

# ---- 6. 等待就绪（用 Get-NetTCPConnection 代替 TcpClient，IPv4/IPv6 都能检测）----
Write-Host "[6/6] 等待服务就绪（最多 90 秒）..." -ForegroundColor Yellow
function Test-Port([int]$port) {
    $listening = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
    return [bool]$listening
}

$status = @{ 7880 = $false; 18080 = $false; 5173 = $false }
$deadline = (Get-Date).AddSeconds(90)
while ((Get-Date) -lt $deadline) {
    foreach ($p in @(7880, 18080, 5173)) {
        if (-not $status[$p] -and (Test-Port $p)) {
            $status[$p] = $true
            Write-Host "   ✅ 端口 $p 就绪" -ForegroundColor Green
        }
    }
    $ready = @($status.Values | Where-Object { $_ }).Count
    if ($ready -eq 3) { break }
    Start-Sleep -Milliseconds 500
}

if ($status.Values -contains $false) {
    Write-Host ""
    Write-Host "   ❌ 超时，部分服务未就绪：" -ForegroundColor Red
    foreach ($p in @(7880, 18080, 5173)) {
        if (-not $status[$p]) {
            Write-Host "      端口 $p 未监听" -ForegroundColor Red
        }
    }
    Write-Host ""
    Write-Host "   请查看其他窗口的日志" -ForegroundColor Yellow
    Read-Host "按回车退出"
    exit 1
}

Write-Host ""
Write-Host "✅ 所有服务已就绪" -ForegroundColor Green
Write-Host "   前端:    http://localhost:5173" -ForegroundColor Cyan
Write-Host "   后端:    http://localhost:18080" -ForegroundColor Cyan
Write-Host "   LiveKit: ws://localhost:7880" -ForegroundColor Cyan
Write-Host ""

# ---- 测试 ----
if (-not $SkipTest) {
    $runTest = Read-Host "运行屏幕共享测试？(Y/n)"
    if ($runTest -ne 'n') {
        Write-Host "运行测试..." -ForegroundColor Yellow
        Push-Location "$root\apps\web"
        try { node test-4-screen-share.cjs } finally { Pop-Location }
        $code = $LASTEXITCODE
        Write-Host ""
        if ($code -eq 0) {
            Write-Host "🎉 测试通过" -ForegroundColor Green
        } else {
            Write-Host "❌ 测试失败 (exit $code)" -ForegroundColor Red
        }
    }
}

Write-Host ""
Write-Host "服务在后台运行。停止：.\stop-all.ps1" -ForegroundColor DarkGray
Read-Host "按回车关闭本窗口"
