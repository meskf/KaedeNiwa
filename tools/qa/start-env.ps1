param(
    [int]$BackendPort = 18080,
    [int]$WebPort = 5173,
    [switch]$WithLiveKit,
    [switch]$WithMedia,
    [switch]$Force
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$QaDir = "$Root\tools\qa"
$RunDir = "$QaDir\.run"
$LogDir = "$RunDir\logs"
New-Item -ItemType Directory -Force -Path $RunDir, $LogDir | Out-Null

# -WithMedia 隐含 -WithLiveKit，并强制重启后端注入 LIVEKIT_* env
if ($WithMedia) { $WithLiveKit = $true }

function Test-Listen([int]$port) {
    return [bool](Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)
}
function Stop-Port([int]$port) {
    Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
}
function Wait-Url([string]$url, [int]$timeoutSec = 30) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 300
        try {
            $r = Invoke-WebRequest -Uri $url -TimeoutSec 2 -UseBasicParsing -ErrorAction Stop
            if ($r.StatusCode -ge 200 -and $r.StatusCode -lt 400) { return $true }
        } catch {}
    }
    return $false
}

if ($Force) {
    Write-Host "[start-env] -Force: 清理 $BackendPort / $WebPort / 7880-7882" -ForegroundColor Yellow
    foreach ($p in @($BackendPort, $WebPort, 7880, 7881, 7882)) { Stop-Port $p }
    # QA 专用数据库：每次 -Force 重建，保证"第一个注册用户 = op5"可复现，
    # 并且不污染开发库 kaede.db。
    foreach ($f in @("$RunDir\qa.db", "$RunDir\qa.db-shm", "$RunDir\qa.db-wal")) {
        Remove-Item $f -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Milliseconds 500
}

# -WithMedia 时后端必须重启（env 才能生效），即使端口已在监听
if ($WithMedia -and (Test-Listen $BackendPort)) {
    Write-Host "[start-env] -WithMedia: 强制重启后端以应用 LIVEKIT_* env" -ForegroundColor Yellow
    Stop-Port $BackendPort
    Start-Sleep -Milliseconds 500
}

$serverExe = if ([System.Environment]::OSVersion.Platform -eq 'Win32NT') { 'kaede-server.exe' } else { 'kaede-server' }
$backendPid = $null; $webPid = $null; $livekitPid = $null

if (Test-Listen $BackendPort) {
    $backendPid = (Get-NetTCPConnection -LocalPort $BackendPort -State Listen).OwningProcess | Select-Object -First 1
    Write-Host "[start-env] 复用已有后端 :$BackendPort (PID $backendPid)" -ForegroundColor DarkGray
} else {
    # LiveKit dev 服务器默认 keys = devkey/secret
    $env:LIVEKIT_API_KEY = 'devkey'
    $env:LIVEKIT_API_SECRET = 'secret'
    $env:LIVEKIT_WS_URL = 'ws://127.0.0.1:7880'
    if (-not $WithMedia) {
        # 无 media 模式不强制 LiveKit 起来，但仍给后端 devkey（token 端点可以 200）
        $env:KAEDE_SKIP_LIVEKIT_ENV_CHECK = '1'
    } else {
        Remove-Item Env:KAEDE_SKIP_LIVEKIT_ENV_CHECK -ErrorAction SilentlyContinue
    }
    $env:KAEDE_BIND = "127.0.0.1:$BackendPort"
    $env:RUST_LOG = "info,kaede_server=debug"
    # 测试环境批量注册/发送会撞 per-IP/per-uid 限流。
    # 仅 dev-mode 构建读取该变量；生产构建 cfg!(feature="dev-mode") 为 false。
    $env:KAEDE_TEST_MODE = '1'
    # QA 独立数据库（相对仓库根目录；后端 WorkingDirectory=$Root）
    $env:KAEDE_DB = 'sqlite://tools/qa/.run/qa.db?mode=rwc'
    # 管理面板：测试用固定 token，供 e2e 做 op 角色编排（仅 localhost）
    $env:KAEDE_ADMIN_ENABLED = '1'
    $env:KAEDE_ADMIN_BIND = '127.0.0.1:18081'
    $env:KAEDE_ADMIN_TOKEN = 'qa-admin-token'

    Write-Host "[start-env] cargo build -p kaede-server --features dev-mode ..." -ForegroundColor Cyan
    Push-Location $Root
    try {
        $buildLog = "$LogDir\build.log"
        cargo build -p kaede-server --features dev-mode 2>&1 | Tee-Object -FilePath $buildLog | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Get-Content $buildLog -Tail 40 | Out-Host
            throw "cargo build failed: $LASTEXITCODE"
        }
    } finally { Pop-Location }

    $proc = Start-Process -FilePath "$Root\target\debug\$serverExe" `
        -WorkingDirectory $Root `
        -RedirectStandardOutput "$LogDir\backend.log" `
        -RedirectStandardError "$LogDir\backend.err.log" `
        -WindowStyle Hidden -PassThru
    $backendPid = $proc.Id
    if (-not (Wait-Url "http://127.0.0.1:$BackendPort/health" 40)) {
        throw "backend :$BackendPort health timeout, see $LogDir\backend.err.log"
    }
    Write-Host "[start-env] 后端就绪 PID=$backendPid (media=$WithMedia)" -ForegroundColor Green
}

if ($WithLiveKit) {
    if (Test-Listen 7880) {
        Write-Host "[start-env] 复用 LiveKit :7880" -ForegroundColor DarkGray
        $livekitPid = (Get-NetTCPConnection -LocalPort 7880 -State Listen).OwningProcess | Select-Object -First 1
    } else {
        $lkWin = Join-Path $Root "tools\livekit\livekit-server.exe"; $lkNix = Join-Path $Root "tools\livekit\livekit-server"; $lk = if (Test-Path $lkWin) { $lkWin } else { $lkNix }
        if (-not (Test-Path $lk)) { throw "missing $lk" }
        # 用 QA 配置（keys=devkey/secret，departure_timeout=2s 快速清理幽灵 participant）
        $lkCfg = Join-Path $QaDir "livekit-qa.yaml"
        $proc = Start-Process -FilePath $lk -ArgumentList '--config', $lkCfg -WindowStyle Hidden `
            -RedirectStandardOutput "$LogDir\livekit.log" `
            -RedirectStandardError "$LogDir\livekit.err.log" -PassThru
        $livekitPid = $proc.Id
        if (-not (Test-Listen 7880)) {
            $deadline = (Get-Date).AddSeconds(15)
            while ((Get-Date) -lt $deadline -and -not (Test-Listen 7880)) { Start-Sleep -Milliseconds 200 }
        }
        if (-not (Test-Listen 7880)) { throw "LiveKit :7880 未监听" }
        Write-Host "[start-env] LiveKit 就绪 PID=$livekitPid" -ForegroundColor Green
    }
}

if (Test-Listen $WebPort) {
    $webPid = (Get-NetTCPConnection -LocalPort $WebPort -State Listen).OwningProcess | Select-Object -First 1
    Write-Host "[start-env] 复用已有前端 :$WebPort (PID $webPid)" -ForegroundColor DarkGray
} else {
    $webScript = "$LogDir\run-web.ps1"
    @"
Set-Location '$Root\apps\web'
`$env:KAEDE_HTTP = '1'
npm run dev -- --host=127.0.0.1 --port=$WebPort
"@ | Set-Content $webScript -Encoding UTF8
    $proc = Start-Process -FilePath "pwsh" `
        -ArgumentList @('-NoProfile', '-File', "`"$webScript`"") `
        -RedirectStandardOutput "$LogDir\web.log" `
        -RedirectStandardError "$LogDir\web.err.log" `
        -WindowStyle Hidden -PassThru
    $webPid = $proc.Id
    if (-not (Wait-Url "http://127.0.0.1:$WebPort/" 90)) {
        throw "vite :$WebPort timeout, see $LogDir\web.err.log"
    }
    Write-Host "[start-env] 前端就绪 PID=$webPid" -ForegroundColor Green
}

$state = [ordered]@{
    backend   = [ordered]@{ pid = $backendPid; port = $BackendPort; url = "http://127.0.0.1:$BackendPort" }
    web       = [ordered]@{ pid = $webPid;     port = $WebPort;     url = "http://127.0.0.1:$WebPort" }
    livekit   = if ($livekitPid) { [ordered]@{ pid = $livekitPid; port = 7880; url = 'ws://127.0.0.1:7880' } } else { $null }
    withMedia = [bool]$WithMedia
    logDir    = $LogDir
    startedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
}
$state | ConvertTo-Json -Depth 6 | Set-Content "$RunDir\state.json" -Encoding UTF8
$state | ConvertTo-Json -Depth 6
