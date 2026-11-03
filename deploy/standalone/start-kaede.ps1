# KaedeNiwa 独立服务端（Windows）启动器
# 用法：双击 start-kaede.cmd，或 powershell -File start-kaede.ps1
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$webPort = 18080
$livekitPort = 7880
New-Item -ItemType Directory -Force -Path logs, data | Out-Null

function Wait-Port([string]$Target, [int]$Port, [int]$TimeoutSec) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $c = New-Object Net.Sockets.TcpClient
        try { $c.Connect($Target, $Port); $c.Close(); return $true } catch { } finally { $c.Dispose() }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Wait-Url([string]$Url, [int]$TimeoutSec) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-WebRequest $Url -UseBasicParsing -TimeoutSec 2
            if ($r.StatusCode -eq 200) { return $true }
        } catch { }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

# LiveKit 与 server 必须使用同一组密钥（也可同时修改 livekit.yaml）
$env:KAEDE_BIND = "0.0.0.0:$webPort"
$env:KAEDE_DB = 'sqlite://kaede.db?mode=rwc'
$env:KAEDE_DATA_DIR = (Join-Path $PWD 'data')
$env:KAEDE_CORS_ORIGINS = '*'
$env:LIVEKIT_API_KEY = 'devkey'
$env:LIVEKIT_API_SECRET = '12345678901234567890123456789012'
# auto：按访问 Host 推导 ws://<host>:7880（局域网/公网 IP 都能用）
$env:LIVEKIT_WS_URL = 'auto'
$env:KAEDE_LIVEKIT_HTTP_URL = "http://127.0.0.1:$livekitPort"
$env:RUST_LOG = 'info'

Write-Host '[kaede] 启动 LiveKit...' -ForegroundColor Cyan
$lk = Start-Process -FilePath (Join-Path $PWD 'livekit-server.exe') `
    -ArgumentList '--config', (Join-Path $PWD 'livekit.yaml') `
    -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $PWD 'logs\livekit.log') `
    -RedirectStandardError (Join-Path $PWD 'logs\livekit.err.log') `
    -PassThru
$lk.Id | Set-Content -Path '.run-livekit.pid'

if (-not (Wait-Port '127.0.0.1' $livekitPort 60)) {
    Write-Host '[kaede] LiveKit 启动失败，请看 logs/livekit.err.log' -ForegroundColor Red
    exit 1
}

Write-Host '[kaede] 启动服务端...' -ForegroundColor Cyan
$sv = Start-Process -FilePath (Join-Path $PWD 'kaede-server.exe') `
    -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $PWD 'logs\server.log') `
    -RedirectStandardError (Join-Path $PWD 'logs\server.err.log') `
    -PassThru
$sv.Id | Set-Content -Path '.run-server.pid'

if (-not (Wait-Url "http://127.0.0.1:$webPort/ready" 90)) {
    Write-Host '[kaede] 服务端启动失败，请看 logs/server.err.log' -ForegroundColor Red
    exit 1
}

Write-Host ''
Write-Host ("[kaede] 已启动：http://127.0.0.1:{0}" -f $webPort) -ForegroundColor Green
Write-Host '首个注册的账号自动成为服主（可建频道）；之后注册的账号是访客。' -ForegroundColor DarkGray
Write-Host '局域网访问：把 127.0.0.1 换成这台机器的 IP；防火墙需放行 TCP 18080/7880、TCP 7881、UDP 7882。' -ForegroundColor DarkGray
Write-Host '停止服务：双击 stop-kaede.cmd' -ForegroundColor DarkGray
if ($env:KAEDE_NO_BROWSER -ne '1') {
    Start-Process "http://127.0.0.1:$webPort/"
}
