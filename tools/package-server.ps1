param(
    [string]$Version = '0.1.1'
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$OutDir = Join-Path $Root "release\KaedeNiwa-server-$Version"
$Zip = Join-Path $Root "release\KaedeNiwa-server-$Version.zip"

$serverExe = Join-Path $Root 'target\release\kaede-server.exe'
$livekitExe = Join-Path $Root 'tools\livekit\livekit-server.exe'
$webDist = Join-Path $Root 'apps\web\dist'

foreach ($p in @($serverExe, $livekitExe, (Join-Path $webDist 'index.html'))) {
    if (-not (Test-Path $p)) { throw "缺少构建产物: $p" }
}

if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
if (Test-Path $Zip) { Remove-Item $Zip -Force }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Copy-Item $serverExe (Join-Path $OutDir 'kaede-server.exe')
Copy-Item $livekitExe (Join-Path $OutDir 'livekit-server.exe')
Copy-Item $webDist (Join-Path $OutDir 'web') -Recurse
Copy-Item (Join-Path $Root 'deploy\standalone\*') $OutDir -Recurse

$hash = (Get-FileHash (Join-Path $OutDir 'kaede-server.exe') -Algorithm SHA256).Hash
"server_sha256=$hash`nversion=$Version`nbuilt=$(Get-Date -Format o)" |
    Set-Content (Join-Path $OutDir 'BUILD-INFO.txt') -Encoding UTF8

Compress-Archive -Path (Join-Path $OutDir '*') -DestinationPath $Zip -CompressionLevel Optimal
$zipMb = [math]::Round((Get-Item $Zip).Length / 1MB, 1)
Write-Host "已生成: $Zip ($zipMb MB)" -ForegroundColor Green
