# KaedeNiwa 完整测试套件（自包含，自动拉起所需服务）
# 用法：
#   .\run-all-tests.ps1                      # 快速（可能因数据库污染失败）
#   .\run-all-tests.ps1 -FreshEachMediaTest  # 稳定（每个 media 测试前清库重启后端）

param(
    [switch]$FreshEachMediaTest
)

$ErrorActionPreference = 'Continue'
$root = "D:\dev2\kaedeniwa"
$web = "$root\apps\web"
$lkExe = "$root\tools\livekit\livekit-server.exe"

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  KaedeNiwa 完整测试套件" -ForegroundColor Cyan
if ($FreshEachMediaTest) {
    Write-Host "  模式: FreshEachMediaTest（每 media 测试前清库重启）" -ForegroundColor Yellow
}
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# ==========================================
# 工具：端口探测
# ==========================================
function Test-Port([int]$port) {
    return [bool](Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)
}

function Wait-Port([int]$port, [int]$timeoutSec = 30) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        if (Test-Port $port) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

# ==========================================
# 确保所有服务就绪
# ==========================================
Write-Host "[服务] 检查 Vite (5173)、LiveKit (7880)、后端 (18080)..." -ForegroundColor Cyan

# --- LiveKit ---
if (-not (Test-Port 7880)) {
    if (-not (Test-Path $lkExe)) {
        Write-Host "   ❌ 缺少 $lkExe，请先下载 LiveKit" -ForegroundColor Red
        Read-Host "按回车退出"; exit 1
    }
    Write-Host "   ▶ 启动 LiveKit..." -ForegroundColor Yellow
    Start-Process -FilePath $lkExe -ArgumentList '--dev' -WindowStyle Normal
    if (-not (Wait-Port 7880 20)) {
        Write-Host "   ❌ LiveKit 启动超时" -ForegroundColor Red
        Read-Host "按回车退出"; exit 1
    }
}
Write-Host "   ✅ LiveKit (7880) 就绪" -ForegroundColor Green

# --- 后端 ---
Remove-Item Env:KAEDE_SKIP_LIVEKIT_ENV_CHECK -ErrorAction SilentlyContinue
$env:LIVEKIT_API_KEY = 'devkey'
$env:LIVEKIT_API_SECRET = 'secret'
$env:LIVEKIT_WS_URL = 'ws://127.0.0.1:7880'
$env:RUST_LOG = 'info,kaede_server=debug'

    Write-Host "   重建 wasm（客户端 Rust 改动）..." -ForegroundColor Cyan
    Push-Location (Join-Path (Split-Path $PSScriptRoot -Parent) "apps\web")
    npm run build:wasm 2>&1 | Out-Null
    $wasmExit = $LASTEXITCODE
    Pop-Location
    if ($wasmExit -ne 0) {
        Write-Host "   wasm build failed (exit $wasmExit)" -ForegroundColor Red
        exit 1
    }
    Write-Host "   OK wasm" -ForegroundColor Green
Write-Host "   预编译后端..." -ForegroundColor DarkGray
Push-Location $root
try { cargo build -p kaede-server --features dev-mode 2>&1 | Out-Null } finally { Pop-Location }

if (-not (Test-Port 18080)) {
    Write-Host "   ▶ 启动后端..." -ForegroundColor Yellow
    Start-Process -FilePath "$root\target\debug\kaede-server.exe" `
        -WorkingDirectory $root -WindowStyle Normal
    if (-not (Wait-Port 18080 30)) {
        Write-Host "   ❌ 后端启动超时" -ForegroundColor Red
        Read-Host "按回车退出"; exit 1
    }
}
Write-Host "   ✅ 后端 (18080) 就绪" -ForegroundColor Green

# --- Vite ---
if (-not (Test-Port 5173)) {
    Write-Host "   ▶ 启动 Vite (5173)..." -ForegroundColor Yellow
    Start-Process pwsh -ArgumentList '-NoExit', '-Command', "Set-Location '$web'; npm run dev" -WindowStyle Normal
    if (-not (Wait-Port 5173 60)) {
        Write-Host "   ❌ Vite 启动超时" -ForegroundColor Red
        Read-Host "按回车退出"; exit 1
    }
}
Write-Host "   ✅ Vite (5173) 就绪" -ForegroundColor Green
Write-Host ""

# ==========================================
# 重启后端（仅 -FreshEachMediaTest 时用）
# ==========================================
function Restart-Backend {
    Write-Host "   ↻ 清库 + 重启后端..." -ForegroundColor Yellow
    Get-Process kaede-server -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 2
    Remove-Item "$root\kaede.db*" -Force -ErrorAction SilentlyContinue
    Start-Process -FilePath "$root\target\debug\kaede-server.exe" `
        -WorkingDirectory $root -WindowStyle Normal
    return (Wait-Port 18080 30)
}

# ==========================================
# 跑测试
# ==========================================
$tests = @(
    @{ name = 'test-1 自动拉人';   file = 'test-1-auto-join.cjs';         media = $false },
    @{ name = 'test-2 频道隔离';   file = 'test-2-channel-isolation.cjs'; media = $false },
    @{ name = 'test-3 断线重连';   file = 'test-3-reconnect.cjs';         media = $false },
    @{ name = 'test-4 屏幕共享';   file = 'test-4-screen-share.cjs';      media = $true },
    @{ name = 'test-5 麦克风+降噪'; file = 'test-5-mic-rnnoise.cjs';       media = $true },
    @{ name = 'test-6 画质切换';   file = 'test-6-quality.cjs';           media = $true },
    @{ name = 'test-7 多人测试';   file = 'test-7-multi-user.cjs';        media = $true },
    @{ name = 'test-8 10用户规模';  file = 'test-8-ten-users.cjs';         media = $true },
    @{ name = 'test-9 Authority 退出'; file = 'test-9-authority-leave.cjs'; media = $true }
    @{ name = 'test-10 Commit 补发'; file = 'test-10-commit-replay.cjs'; media = $true }
)

$results = @()
Push-Location $web
try {
    foreach ($t in $tests) {
        if ($FreshEachMediaTest -and $t.media) {
            if (-not (Restart-Backend)) {
                Write-Host "   ❌ 后端重启超时，跳过 $($t.name)" -ForegroundColor Red
                $results += [pscustomobject]@{ Test = $t.name; Result = 'FAIL'; Time = 'timeout' }
                continue
            }
        }
        Write-Host ""
        Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Magenta
        Write-Host "▶ $($t.name)" -ForegroundColor Magenta
        Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Magenta
        $start = Get-Date
        node $t.file
        $code = $LASTEXITCODE
        $elapsed = [math]::Round(((Get-Date) - $start).TotalSeconds, 1)
        $results += [pscustomobject]@{
            Test = $t.name
            Result = if ($code -eq 0) { 'PASS' } else { 'FAIL' }
            Time = "${elapsed}s"
        }
    }
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  测试结果汇总" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
$results | Format-Table -AutoSize

$passed = ($results | Where-Object { $_.Result -eq 'PASS' }).Count
$total = $results.Count
Write-Host ""
if ($passed -eq $total) {
    Write-Host "🎉 全部通过 ($passed/$total)" -ForegroundColor Green
} else {
    Write-Host "⚠️ $passed/$total 通过" -ForegroundColor Yellow
    $results | Where-Object { $_.Result -eq 'FAIL' } | ForEach-Object {
        Write-Host "   ❌ $($_.Test)" -ForegroundColor Red
    }
}
Write-Host ""
Read-Host "按回车关闭"


