#requires -Version 7
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

Write-Host "===== KaedeNiwa 客户端打包 =====" -ForegroundColor Cyan

# 1. 检查 web/dist
$dist = 'apps\web\dist'
if (-not (Test-Path "$dist\index.html")) {
  throw "缺少 $dist\index.html，请先 cd apps\web; npm run build"
}
$distMtime = (Get-Item "$dist\index.html").LastWriteTime
$newestSrc = Get-ChildItem apps\web\src -Recurse -File -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($newestSrc -and $newestSrc.LastWriteTime -gt $distMtime) {
  Write-Host "⚠️ 前端源码比 dist 新，建议先 cd apps\web; npm run build 再打客户端" -ForegroundColor Yellow
}

# 2. 检查 wasm
if (-not (Test-Path 'apps\web\src\wasm\pkg\kaede_client_wasm_bg.wasm')) {
  throw "缺少 wasm，请先 cd apps\web; npm run build:wasm"
}

# 3. 检查 electron-builder
if (-not (Test-Path 'apps\desktop\node_modules\electron-builder')) {
  throw "缺少 electron-builder，请先 cd apps\desktop; npm install -D electron-builder"
}

# 4. 打包
Push-Location apps\desktop
try {
  Write-Host "`n运行 electron-builder（首次会下载 NSIS 资源，约 30-60s）..." -ForegroundColor Cyan
  npx electron-builder --win nsis portable
  if ($LASTEXITCODE -ne 0) { throw "electron-builder 失败（退出码 $LASTEXITCODE）" }
} finally {
  Pop-Location
}

# 5. 验证产物
$out = 'release\desktop'
if (-not (Test-Path $out)) { throw "未找到产物目录 $out" }
$artifacts = Get-ChildItem $out -File | Where-Object { $_.Extension -in '.exe' }
Write-Host "`n===== 产物 =====" -ForegroundColor Green
$artifacts | Select-Object Name, @{n='MB';e={[math]::Round($_.Length/1MB,1)}}, LastWriteTime |
  Format-Table -AutoSize
if (-not $artifacts) { throw "未生成任何安装包" }

Write-Host "`n✅ 客户端打包完成" -ForegroundColor Green
