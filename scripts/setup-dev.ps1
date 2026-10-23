# KaedeNiwa 首次 clone 后的一键环境配置。
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Split-Path -Parent $PSScriptRoot)).Path
$webrtc = Join-Path $root '_webrtc'
Write-Host "KaedeNiwa dev setup: $root" -ForegroundColor Cyan
if (-not (Test-Path $webrtc)) {
    Write-Host "警告: $webrtc 不存在，media 构建会失败" -ForegroundColor Yellow
    Write-Host "按 README「_webrtc/ 依赖」重新生成后重跑" -ForegroundColor Yellow
    exit 1
}
$global = Join-Path $HOME '.cargo\config.toml'
$line = "LK_CUSTOM_WEBRTC = `"$($webrtc -replace '\\','\\\\')`""
$dir = Split-Path -Parent $global
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
if ((Test-Path $global) -and (Select-String -Path $global -Pattern 'LK_CUSTOM_WEBRTC' -Quiet)) {
    Write-Host "已配置，跳过: $global" -ForegroundColor Green
    exit 0
}
$content = if (Test-Path $global) { Get-Content $global -Raw } else { '' }
$append = if ($content -match "(?m)^\[env\]") { "`n$line" } else { "`n[env]`n$line" }
Add-Content $global $append
Write-Host "已追加到 $global" -ForegroundColor Green
