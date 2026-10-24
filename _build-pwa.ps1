# 一键 build PWA：dx build + postbuild 复制 PWA 文件
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$env:CARGO_UNSTABLE_EMBED_METADATA = 'true'

Write-Host '[1/2] dx build --package kaede-wasm --platform web'
dx build --package kaede-wasm --platform web
if ($LASTEXITCODE -ne 0) { Write-Host 'dx build 失败'; exit 1 }

Write-Host ''
Write-Host '[2/2] postbuild：复制 PWA 文件到 dist'
node scripts/postbuild.cjs
if ($LASTEXITCODE -ne 0) { Write-Host 'postbuild 失败'; exit 1 }

Write-Host ''
Write-Host '✓ PWA build 完成，dist 位置：'
Write-Host '  target\dx\kaede-wasm\debug\web\public'
