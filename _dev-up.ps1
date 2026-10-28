Set-Location D:\dev2\kaedeniwa
$ErrorActionPreference = 'Continue'
$root = 'D:\dev2\kaedeniwa'
$logs = Join-Path $root 'logs'
New-Item -ItemType Directory -Force -Path $logs | Out-Null

Get-Process kaede-server,livekit-server -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-NetTCPConnection -LocalPort 18080,7880 -State Listen -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
Start-Sleep 1

$lk = Join-Path $root 'tools\livekit\livekit-server.exe'
if (Test-Path $lk) {
    Start-Process -FilePath $lk -ArgumentList '--dev' `
        -WorkingDirectory (Split-Path $lk) `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $logs 'lk.out') `
        -RedirectStandardError  (Join-Path $logs 'lk.err')
    Write-Host '[dev-up] livekit started'
} else {
    Write-Host '[dev-up] WARN no livekit-server.exe'
}

$srv = Join-Path $root 'target\debug\kaede-server.exe'
if (-not (Test-Path $srv)) { Write-Host "[dev-up] ERR no binary at $srv"; return }
Start-Process -FilePath $srv `
    -WorkingDirectory $root `
    -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $logs 'server.out') `
    -RedirectStandardError  (Join-Path $logs 'server.err') `
    -Environment @{
        LIVEKIT_API_KEY    = 'devkey'
        LIVEKIT_API_SECRET = 'secret'
        LIVEKIT_WS_URL     = 'ws://127.0.0.1:7880'
        RUST_LOG           = 'info,kaede_server=debug'
    }
Write-Host '[dev-up] server started'

$deadline = (Get-Date).AddSeconds(60)
$last = (Get-Date)
while ((Get-Date) -lt $deadline) {
    $ready = @()
    foreach ($p in 7880,18080,5173) {
        if (Get-NetTCPConnection -LocalPort $p -State Listen -ErrorAction SilentlyContinue) { $ready += $p }
    }
    if ($ready.Count -eq 3) { Write-Host '  OK all ports up'; return }
    if (((Get-Date) - $last).TotalSeconds -ge 8) {
        Write-Host "   [$([DateTime]::Now.ToString('HH:mm:ss'))] ready: $($ready -join ',')"
        $last = Get-Date
    }
    Start-Sleep -Seconds 2
}
Write-Host '  TIMEOUT'
Write-Host '=== server.err tail ==='
Get-Content (Join-Path $logs 'server.err') -Tail 40 -ErrorAction SilentlyContinue
Write-Host '=== server.out tail ==='
Get-Content (Join-Path $logs 'server.out') -Tail 20 -ErrorAction SilentlyContinue
Write-Host '=== lk.err tail ==='
Get-Content (Join-Path $logs 'lk.err') -Tail 30 -ErrorAction SilentlyContinue
