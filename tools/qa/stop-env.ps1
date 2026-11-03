param([switch]$KeepLogs)
$ErrorActionPreference = 'Continue'
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$QaDir = "$Root\tools\qa"
$RunDir = "$QaDir\.run"
$statePath = "$RunDir\state.json"

$ports = @(18080, 5173)
if (Test-Path $statePath) {
    $state = Get-Content $statePath -Raw | ConvertFrom-Json
    foreach ($svc in @('backend','web','livekit')) {
        $s = $state.$svc
        if ($s -and $s.pid) {
            Stop-Process -Id $s.pid -Force -ErrorAction SilentlyContinue
            Write-Host "[stop-env] 已停 $svc (PID $($s.pid))" -ForegroundColor Green
        }
    }
    if ($state.backend.port) { $ports = @($state.backend.port, $state.web.port, 7880, 7881, 7882) }
}

foreach ($p in $ports) {
    Get-NetTCPConnection -LocalPort $p -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object {
            Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
            Write-Host "[stop-env] 兜底停端口 $p (PID $($_.OwningProcess))" -ForegroundColor DarkGray
        }
}

if (-not $KeepLogs) { Remove-Item $statePath -Force -ErrorAction SilentlyContinue }
Write-Host "[stop-env] 完成" -ForegroundColor Cyan
