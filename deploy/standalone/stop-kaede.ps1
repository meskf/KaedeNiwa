# 停止 KaedeNiwa 独立服务端
$ErrorActionPreference = 'Continue'
Set-Location $PSScriptRoot

foreach ($f in @('.run-server.pid', '.run-livekit.pid')) {
    if (Test-Path $f) {
        $pidValue = Get-Content $f | Select-Object -First 1
        if ($pidValue) { Stop-Process -Id ([int]$pidValue) -Force -ErrorAction SilentlyContinue }
        Remove-Item $f -Force -ErrorAction SilentlyContinue
    }
}

foreach ($p in @(18080, 7880, 7881, 7882)) {
    Get-NetTCPConnection -LocalPort $p -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
}

Write-Host '[kaede] 已停止。' -ForegroundColor Green
