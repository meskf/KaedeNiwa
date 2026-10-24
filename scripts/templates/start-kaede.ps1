# KaedeNiwa 内测启动器（由 start-kaede.bat 调用）
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$bin = Join-Path $PSScriptRoot 'bin'
$cfg = Join-Path $PSScriptRoot 'config'
$log = Join-Path $PSScriptRoot 'logs'
$gst = Join-Path $PSScriptRoot 'gstreamer'
$envFile = Join-Path $cfg '.env'
$lkYaml = Join-Path $cfg 'livekit.yaml'
$serverExe = Join-Path $bin 'kaede-server.exe'
$lkExe = Join-Path $bin 'livekit-server.exe'
$deskExe = Join-Path $bin 'kaedeniwa.exe'
$serverLog = Join-Path $log 'server.log'
# $lkLog = Join-Path $log 'livekit.log'  # 由 server 内部管理

Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  KaedeNiwa 内测版" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

# 前置检查
foreach ($f in @($serverExe, $deskExe, $envFile, $lkYaml)) {
    if (-not (Test-Path $f)) { Write-Host "❌ 缺失: $f" -ForegroundColor Red; Read-Host "按 Enter 退出"; exit 1 }
}
if (-not (Test-Path $lkExe)) { Write-Host "⚠️ 未包含 livekit-server.exe，将只启动文字聊天" -ForegroundColor Yellow }

# 清理残留
$existing = Get-Process kaede-server, livekit-server, kaedeniwa -EA SilentlyContinue
if ($existing) {
    Write-Host "检测到上次未清理的进程，先关闭..." -ForegroundColor Yellow
    $existing | Stop-Process -Force -EA SilentlyContinue
    Start-Sleep -Milliseconds 800
}

# 加载 .env（只加载 KEY=VALUE，跳过注释和空行）
foreach ($line in Get-Content $envFile -Encoding UTF8) {
    $t = $line.Trim()
    if (-not $t -or $t.StartsWith('#') -or $t -notmatch '=') { continue }
    $k, $v = $t -split '=', 2
    [Environment]::SetEnvironmentVariable($k.Trim(), $v.Trim(), 'Process')
}

# 加入 GStreamer DLL 到 PATH（desktop 依赖）
if (Test-Path "$gst\bin") {
    $env:PATH = "$gst\bin;$gst\lib\gstreamer-1.0;$env:PATH"
    $env:GST_PLUGIN_PATH_1_0 = "$gst\lib\gstreamer-1.0"
}

$procs = @()
try {
# ============================================================
# 局域网模式自动检测（KAEDE_BIND=0.0.0.0:* 时生效）
# 原 .env 默认是 127.0.0.1，内测者若改为 0.0.0.0 则朋友可连。
# 这里自动检测本机局域网 IP，填进 LIVEKIT_WS_URL 和 CORS，用户无需手填。
# ============================================================
$bindIp = ($env:KAEDE_BIND -split ':')[0]
$isLanMode = ($bindIp -eq '0.0.0.0')
$lanIp = $null

if ($isLanMode) {
    $lanIp = (Get-NetIPAddress -AddressFamily IPv4 -PrefixOrigin Dhcp,Manual -ErrorAction SilentlyContinue |
        Where-Object {
            # KaedeNiwa 内测启动器（由 start-kaede.bat 调用）
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$bin = Join-Path $PSScriptRoot 'bin'
$cfg = Join-Path $PSScriptRoot 'config'
$log = Join-Path $PSScriptRoot 'logs'
$gst = Join-Path $PSScriptRoot 'gstreamer'
$envFile = Join-Path $cfg '.env'
$lkYaml = Join-Path $cfg 'livekit.yaml'
$serverExe = Join-Path $bin 'kaede-server.exe'
$lkExe = Join-Path $bin 'livekit-server.exe'
$deskExe = Join-Path $bin 'kaedeniwa.exe'
$serverLog = Join-Path $log 'server.log'
# $lkLog = Join-Path $log 'livekit.log'  # 由 server 内部管理

Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  KaedeNiwa 内测版" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

# 前置检查
foreach ($f in @($serverExe, $deskExe, $envFile, $lkYaml)) {
    if (-not (Test-Path $f)) { Write-Host "❌ 缺失: $f" -ForegroundColor Red; Read-Host "按 Enter 退出"; exit 1 }
}
if (-not (Test-Path $lkExe)) { Write-Host "⚠️ 未包含 livekit-server.exe，将只启动文字聊天" -ForegroundColor Yellow }

# 清理残留
$existing = Get-Process kaede-server, livekit-server, kaedeniwa -EA SilentlyContinue
if ($existing) {
    Write-Host "检测到上次未清理的进程，先关闭..." -ForegroundColor Yellow
    $existing | Stop-Process -Force -EA SilentlyContinue
    Start-Sleep -Milliseconds 800
}

# 加载 .env（只加载 KEY=VALUE，跳过注释和空行）
foreach ($line in Get-Content $envFile -Encoding UTF8) {
    $t = $line.Trim()
    if (-not $t -or $t.StartsWith('#') -or $t -notmatch '=') { continue }
    $k, $v = $t -split '=', 2
    [Environment]::SetEnvironmentVariable($k.Trim(), $v.Trim(), 'Process')
}

# 加入 GStreamer DLL 到 PATH（desktop 依赖）
if (Test-Path "$gst\bin") {
    $env:PATH = "$gst\bin;$gst\lib\gstreamer-1.0;$env:PATH"
    $env:GST_PLUGIN_PATH_1_0 = "$gst\lib\gstreamer-1.0"
}

$procs = @()
try {
    # [1/3] 提示：kaede-server 会自动拉起同目录的 livekit-server.exe
    if (Test-Path $lkExe) {
        Write-Host "[1/3] LiveKit 将由 kaede-server 自动拉起（无需手动启动）" -ForegroundColor Yellow
    } else {
        Write-Host "[1/3] 未包含 livekit-server.exe（仅文字聊天可用）" -ForegroundColor Yellow
    }

    # [2/3] Kaede Server
    Write-Host "[2/3] 启动 Kaede Server (端口 18080)..." -ForegroundColor Yellow
    $srv = Start-Process -FilePath $serverExe -RedirectStandardOutput $serverLog -RedirectStandardError "$serverLog.err" `
        -PassThru -WindowStyle Hidden
    $procs += $srv
    Write-Host "  PID=$($srv.Id)，等待 /ready ..." -ForegroundColor DarkGray

    $ready = $false
    for ($i = 0; $i -lt 60; $i++) {
        if ($srv.HasExited) { throw "Server 启动失败（退出码 $($srv.ExitCode)），看 $serverLog" }
        try {
            $r = Invoke-WebRequest -Uri 'http://127.0.0.1:18080/ready' -UseBasicParsing -TimeoutSec 2
            if ($r.StatusCode -eq 200) { $ready = $true; break }
        } catch { }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw "Server 30 秒内未 ready，看 $serverLog" }
    Write-Host "  ✓ Server ready: http://127.0.0.1:18080" -ForegroundColor Green

    # [3/3] Desktop（前台）
    Write-Host "[3/3] 打开 KaedeNiwa 桌面客户端..." -ForegroundColor Yellow
    $desk = Start-Process -FilePath $deskExe -PassThru
    $procs += $desk
    Write-Host "  ✓ Desktop PID=$($desk.Id)，关闭窗口即退出全部服务" -ForegroundColor Green
    Write-Host ""
    Write-Host "服务器地址（登录页预填）: http://127.0.0.1:18080" -ForegroundColor Cyan
if ($script:IsLanMode) {
    Write-Host "局域网地址（朋友填这个）: http://$($script:LanIp):18080" -ForegroundColor Cyan
}
    Write-Host "日志: $log" -ForegroundColor DarkGray
    Write-Host ""

    Wait-Process -Id $desk.Id
    Write-Host "`n客户端已退出，正在停止服务..." -ForegroundColor Yellow
}
catch {
    Write-Host "`n❌ 错误: $_" -ForegroundColor Red
    Read-Host "按 Enter 退出（服务会被清理）"
}
finally {
    foreach ($p in $procs) {
        if ($p -and -not $p.HasExited) {
            try { Stop-Process -Id $p.Id -Force -EA SilentlyContinue } catch { }
        }
    }
    # 兜底
    Get-Process kaede-server, livekit-server -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
    Write-Host "✓ 全部服务已停止" -ForegroundColor Green
}
.IPAddress -notmatch '^(127\.|169\.254\.)' -and
            # KaedeNiwa 内测启动器（由 start-kaede.bat 调用）
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$bin = Join-Path $PSScriptRoot 'bin'
$cfg = Join-Path $PSScriptRoot 'config'
$log = Join-Path $PSScriptRoot 'logs'
$gst = Join-Path $PSScriptRoot 'gstreamer'
$envFile = Join-Path $cfg '.env'
$lkYaml = Join-Path $cfg 'livekit.yaml'
$serverExe = Join-Path $bin 'kaede-server.exe'
$lkExe = Join-Path $bin 'livekit-server.exe'
$deskExe = Join-Path $bin 'kaedeniwa.exe'
$serverLog = Join-Path $log 'server.log'
# $lkLog = Join-Path $log 'livekit.log'  # 由 server 内部管理

Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  KaedeNiwa 内测版" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

# 前置检查
foreach ($f in @($serverExe, $deskExe, $envFile, $lkYaml)) {
    if (-not (Test-Path $f)) { Write-Host "❌ 缺失: $f" -ForegroundColor Red; Read-Host "按 Enter 退出"; exit 1 }
}
if (-not (Test-Path $lkExe)) { Write-Host "⚠️ 未包含 livekit-server.exe，将只启动文字聊天" -ForegroundColor Yellow }

# 清理残留
$existing = Get-Process kaede-server, livekit-server, kaedeniwa -EA SilentlyContinue
if ($existing) {
    Write-Host "检测到上次未清理的进程，先关闭..." -ForegroundColor Yellow
    $existing | Stop-Process -Force -EA SilentlyContinue
    Start-Sleep -Milliseconds 800
}

# 加载 .env（只加载 KEY=VALUE，跳过注释和空行）
foreach ($line in Get-Content $envFile -Encoding UTF8) {
    $t = $line.Trim()
    if (-not $t -or $t.StartsWith('#') -or $t -notmatch '=') { continue }
    $k, $v = $t -split '=', 2
    [Environment]::SetEnvironmentVariable($k.Trim(), $v.Trim(), 'Process')
}

# 加入 GStreamer DLL 到 PATH（desktop 依赖）
if (Test-Path "$gst\bin") {
    $env:PATH = "$gst\bin;$gst\lib\gstreamer-1.0;$env:PATH"
    $env:GST_PLUGIN_PATH_1_0 = "$gst\lib\gstreamer-1.0"
}

$procs = @()
try {
    # [1/3] 提示：kaede-server 会自动拉起同目录的 livekit-server.exe
    if (Test-Path $lkExe) {
        Write-Host "[1/3] LiveKit 将由 kaede-server 自动拉起（无需手动启动）" -ForegroundColor Yellow
    } else {
        Write-Host "[1/3] 未包含 livekit-server.exe（仅文字聊天可用）" -ForegroundColor Yellow
    }

    # [2/3] Kaede Server
    Write-Host "[2/3] 启动 Kaede Server (端口 18080)..." -ForegroundColor Yellow
    $srv = Start-Process -FilePath $serverExe -RedirectStandardOutput $serverLog -RedirectStandardError "$serverLog.err" `
        -PassThru -WindowStyle Hidden
    $procs += $srv
    Write-Host "  PID=$($srv.Id)，等待 /ready ..." -ForegroundColor DarkGray

    $ready = $false
    for ($i = 0; $i -lt 60; $i++) {
        if ($srv.HasExited) { throw "Server 启动失败（退出码 $($srv.ExitCode)），看 $serverLog" }
        try {
            $r = Invoke-WebRequest -Uri 'http://127.0.0.1:18080/ready' -UseBasicParsing -TimeoutSec 2
            if ($r.StatusCode -eq 200) { $ready = $true; break }
        } catch { }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw "Server 30 秒内未 ready，看 $serverLog" }
    Write-Host "  ✓ Server ready: http://127.0.0.1:18080" -ForegroundColor Green

    # [3/3] Desktop（前台）
    Write-Host "[3/3] 打开 KaedeNiwa 桌面客户端..." -ForegroundColor Yellow
    $desk = Start-Process -FilePath $deskExe -PassThru
    $procs += $desk
    Write-Host "  ✓ Desktop PID=$($desk.Id)，关闭窗口即退出全部服务" -ForegroundColor Green
    Write-Host ""
    Write-Host "服务器地址（登录页预填）: http://127.0.0.1:18080" -ForegroundColor Cyan
if ($script:IsLanMode) {
    Write-Host "局域网地址（朋友填这个）: http://$($script:LanIp):18080" -ForegroundColor Cyan
}
    Write-Host "日志: $log" -ForegroundColor DarkGray
    Write-Host ""

    Wait-Process -Id $desk.Id
    Write-Host "`n客户端已退出，正在停止服务..." -ForegroundColor Yellow
}
catch {
    Write-Host "`n❌ 错误: $_" -ForegroundColor Red
    Read-Host "按 Enter 退出（服务会被清理）"
}
finally {
    foreach ($p in $procs) {
        if ($p -and -not $p.HasExited) {
            try { Stop-Process -Id $p.Id -Force -EA SilentlyContinue } catch { }
        }
    }
    # 兜底
    Get-Process kaede-server, livekit-server -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
    Write-Host "✓ 全部服务已停止" -ForegroundColor Green
}
.InterfaceAlias -notmatch 'Loopback|vEthernet|WSL|Hyper-V|Docker'
        } |
        Select-Object -First 1 -ExpandProperty IPAddress)
    if (-not $lanIp) {
        Write-Host "  ⚠️ 检测不到局域网 IP，回退到 127.0.0.1（仅本机）" -ForegroundColor Yellow
        $lanIp = '127.0.0.1'
        $isLanMode = $false
    } else {
        Write-Host "  ✓ 局域网模式，本机 IP = $lanIp" -ForegroundColor Green
    }
}

# LIVEKIT_WS_URL：局域网模式用本机 IP，否则用 env 或 127.0.0.1
if ($isLanMode) {
    $env:LIVEKIT_WS_URL = "ws://${lanIp}:18082"
} elseif (-not $env:LIVEKIT_WS_URL) {
    $env:LIVEKIT_WS_URL = "ws://127.0.0.1:18082"
}
Write-Host "  LIVEKIT_WS_URL = $env:LIVEKIT_WS_URL"

# KAEDE_CORS_ORIGINS：始终包含 localhost，局域网追加本机 IP
$baseOrigins = "http://127.0.0.1:18080,http://localhost:18080,http://127.0.0.1:18085,http://localhost:18085"
if ($isLanMode) {
    $lanOrigin = "http://${lanIp}:18080"
    if ($env:KAEDE_CORS_ORIGINS) {
        if ($env:KAEDE_CORS_ORIGINS -notmatch [regex]::Escape($lanOrigin)) {
            $env:KAEDE_CORS_ORIGINS = "$($env:KAEDE_CORS_ORIGINS),$lanOrigin"
        }
    } else {
        $env:KAEDE_CORS_ORIGINS = "$baseOrigins,$lanOrigin"
    }
} elseif (-not $env:KAEDE_CORS_ORIGINS) {
    $env:KAEDE_CORS_ORIGINS = $baseOrigins
}
Write-Host "  KAEDE_CORS_ORIGINS = $env:KAEDE_CORS_ORIGINS"

# 记录给后面打印地址用
$script:IsLanMode = $isLanMode
$script:LanIp = $lanIp
# ============================================================

    # [1/3] 提示：kaede-server 会自动拉起同目录的 livekit-server.exe
    if (Test-Path $lkExe) {
        Write-Host "[1/3] LiveKit 将由 kaede-server 自动拉起（无需手动启动）" -ForegroundColor Yellow
    } else {
        Write-Host "[1/3] 未包含 livekit-server.exe（仅文字聊天可用）" -ForegroundColor Yellow
    }

    # [2/3] Kaede Server
    Write-Host "[2/3] 启动 Kaede Server (端口 18080)..." -ForegroundColor Yellow
    $srv = Start-Process -FilePath $serverExe -RedirectStandardOutput $serverLog -RedirectStandardError "$serverLog.err" `
        -PassThru -WindowStyle Hidden
    $procs += $srv
    Write-Host "  PID=$($srv.Id)，等待 /ready ..." -ForegroundColor DarkGray

    $ready = $false
    for ($i = 0; $i -lt 60; $i++) {
        if ($srv.HasExited) { throw "Server 启动失败（退出码 $($srv.ExitCode)），看 $serverLog" }
        try {
            $r = Invoke-WebRequest -Uri 'http://127.0.0.1:18080/ready' -UseBasicParsing -TimeoutSec 2
            if ($r.StatusCode -eq 200) { $ready = $true; break }
        } catch { }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw "Server 30 秒内未 ready，看 $serverLog" }
    Write-Host "  ✓ Server ready: http://127.0.0.1:18080" -ForegroundColor Green

    # [3/3] Desktop（前台）
    Write-Host "[3/3] 打开 KaedeNiwa 桌面客户端..." -ForegroundColor Yellow
    $desk = Start-Process -FilePath $deskExe -PassThru
    $procs += $desk
    Write-Host "  ✓ Desktop PID=$($desk.Id)，关闭窗口即退出全部服务" -ForegroundColor Green
    Write-Host ""
    Write-Host "服务器地址（登录页预填）: http://127.0.0.1:18080" -ForegroundColor Cyan
if ($script:IsLanMode) {
    Write-Host "局域网地址（朋友填这个）: http://$($script:LanIp):18080" -ForegroundColor Cyan
}
    Write-Host "日志: $log" -ForegroundColor DarkGray
    Write-Host ""

    Wait-Process -Id $desk.Id
    Write-Host "`n客户端已退出，正在停止服务..." -ForegroundColor Yellow
}
catch {
    Write-Host "`n❌ 错误: $_" -ForegroundColor Red
    Read-Host "按 Enter 退出（服务会被清理）"
}
finally {
    foreach ($p in $procs) {
        if ($p -and -not $p.HasExited) {
            try { Stop-Process -Id $p.Id -Force -EA SilentlyContinue } catch { }
        }
    }
    # 兜底
    Get-Process kaede-server, livekit-server -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
    Write-Host "✓ 全部服务已停止" -ForegroundColor Green
}
