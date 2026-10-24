# KaedeNiwa 内测开箱即用打包脚本
# 用法: pwsh -File scripts/bundle-release.ps1 -Version v0.0.4-beta.1
param(
    [string]$Version = "v0.0.4-beta.1",
    [string]$LiveKitExe = "tools\livekit\livekit-server.exe",
    [switch]$SkipIcon,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$tag = "kaedeniwa-beta-$Version"
$out = "release\$tag"
Write-Host "目标: $out" -ForegroundColor Cyan
New-Item -ItemType Directory -Path $out -Force | Out-Null
New-Item -ItemType Directory -Path "$out\bin","$out\config","$out\data","$out\logs","$out\gstreamer\bin","$out\gstreamer\lib\gstreamer-1.0" -Force | Out-Null

# ---------- [1/6] 图标 ----------
if (-not $SkipIcon) {
    Write-Host "`n[1/6] 重新生成图标..." -ForegroundColor Yellow
    Push-Location scripts\icon
    node generate.mjs
    Pop-Location
}

# ---------- [2/6] 编译 server ----------
if (-not $SkipBuild) {
    Write-Host "`n[2/6] 编译 kaede-server (release)..." -ForegroundColor Yellow
    $env:CARGO_BUILD_JOBS = "2"
    cargo build --release --locked -p kaede-server
    if ($LASTEXITCODE -ne 0) { throw "kaede-server 编译失败" }
}
Copy-Item "target\release\kaede-server.exe" "$out\bin\" -Force
Write-Host "  ok  kaede-server.exe"

# ---------- [3/6] 编译 desktop ----------
if (-not $SkipBuild) {
    Write-Host "`n[3/6] 编译 kaede-desktop (release)..." -ForegroundColor Yellow
    if (-not (Get-Command dx -ErrorAction SilentlyContinue)) { throw "需要 dx (dioxus-cli)" }
    Push-Location crates\kaede-desktop
    $toml = Get-Content Dioxus.toml -Raw
    $toml2 = $toml -replace 'version\s*=\s*"0\.0\.2"', "version = `"$($Version.TrimStart('v'))`""
    if ($toml2 -ne $toml) { $toml2 | Set-Content Dioxus.toml -NoNewline; Write-Host "  Dioxus.toml 版本 → $Version" -ForegroundColor DarkGray }
    dx bundle --platform desktop --release --out-dir ..\..\target\bundle-desktop
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw "desktop bundle 失败" }
    Pop-Location
}

# 优先裸 exe：dx 的 release/windows/app/ 目录
$deskExe = Get-ChildItem "target\dx\kaede-desktop\release\windows\app" -Filter "*.exe" -EA SilentlyContinue |
  Where-Object { $_.Name -notmatch 'setup|uninstall|installer|nsis' } | Select-Object -First 1
if (-not $deskExe) {
    $deskExe = Get-ChildItem "target\dx\kaede-desktop\release" -Filter "*.exe" -Recurse -EA SilentlyContinue |
      Where-Object { $_.Name -notmatch 'setup|uninstall|installer|nsis' } | Select-Object -First 1
}
if (-not $deskExe) {
    $deskExe = Get-ChildItem "target\dx\kaede-desktop\bundle\windows" -Filter "*.exe" -EA SilentlyContinue |
      Where-Object { $_.Name -notmatch 'setup|uninstall|installer|nsis' } | Select-Object -First 1
}
if (-not $deskExe) {
    $deskExe = Get-ChildItem "target\release" -Filter "*.exe" -EA SilentlyContinue |
      Where-Object { $_.Name -match 'kaede-desktop|kaedeniwa' -and $_.Name -notmatch 'setup' } | Select-Object -First 1
}
if (-not $deskExe) { throw "找不到 desktop exe（检查 target\dx\kaede-desktop\release\windows\app）" }
Write-Host "  使用: $($deskExe.FullName)"
Copy-Item $deskExe.FullName "$out\bin\kaedeniwa.exe" -Force
Write-Host "  ok  kaedeniwa.exe  (源: $($deskExe.Name), $([math]::Round($deskExe.Length/1MB,1)) MB)"

# ---------- [4/6] LiveKit + GStreamer DLL ----------
Write-Host "`n[4/6] LiveKit + GStreamer..." -ForegroundColor Yellow
if (Test-Path $LiveKitExe) {
    Copy-Item $LiveKitExe "$out\bin\livekit-server.exe" -Force
    Write-Host "  ok  livekit-server.exe  ($([math]::Round((Get-Item $LiveKitExe).Length/1MB,1)) MB)"
} else {
    Write-Host "  ⚠️ 找不到 $LiveKitExe，跳过（客户端将无语音/屏幕共享）" -ForegroundColor Yellow
}

$gstRoot = $env:GSTREAMER_1_0_ROOT_MSVC_X86_64
if (-not $gstRoot) {
    $gst = Get-Command gst-launch-1.0.exe -EA SilentlyContinue
    if ($gst) { $gstRoot = Split-Path (Split-Path $gst.Source -Parent) -Parent }
}
if ($gstRoot -and (Test-Path "$gstRoot\bin\gstreamer-1.0-0.dll")) {
    Copy-Item "$gstRoot\bin\*.dll" "$out\gstreamer\bin\" -Force
    if (Test-Path "$gstRoot\lib\gstreamer-1.0") {
        Copy-Item "$gstRoot\lib\gstreamer-1.0\*.dll" "$out\gstreamer\lib\gstreamer-1.0\" -Force
    }
    $gstSize = ((Get-ChildItem "$out\gstreamer" -Recurse -File | Measure-Object Length -Sum).Sum)/1MB
    Write-Host ("  ok  GStreamer DLL  {0:N0} 文件  {1:N1} MB" -f (Get-ChildItem "$out\gstreamer" -Recurse -File).Count, $gstSize)
} else {
    Write-Host "  ⚠️ 未找到 GStreamer（desktop 语音/屏幕共享需要它）" -ForegroundColor Yellow
    Write-Host "    参考: https://gstreamer.freedesktop.org/download/#windows" -ForegroundColor DarkGray
}

# ---------- [5/6] 生成配置（随机 key/secret） ----------
Write-Host "`n[5/6] 生成 config/.env + livekit.yaml..." -ForegroundColor Yellow
$rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
function RandomHex([int]$bytes) {
    $b = New-Object byte[] $bytes; $rng.GetBytes($b)
    -join ($b | ForEach-Object { $_.ToString('x2') })
}
$lkKey = RandomHex 16
$lkSecret = RandomHex 32

@"
# KaedeNiwa 内测配置（自动生成，请勿外传）
# 生成时间: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')

KAEDE_BIND=127.0.0.1:18080
KAEDE_DB=sqlite://./data/kaede.db?mode=rwc
KAEDE_DATA_DIR=./data
KAEDE_SESSION_TTL_HOURS=168
KAEDE_CORS_ORIGINS=http://127.0.0.1:18080,http://localhost:18080
KAEDE_LOG_FORMAT=json
RUST_LOG=info,kaede_server=info

LIVEKIT_API_KEY=$lkKey
LIVEKIT_API_SECRET=$lkSecret
LIVEKIT_WS_URL=ws://127.0.0.1:7880

KAEDE_ADMIN_ENABLED=0
KAEDE_ADMIN_BIND=127.0.0.1:18081
KAEDE_ADMIN_TOKEN=
"@ | Set-Content "$out\config\.env" -Encoding UTF8 -NoNewline

@"
# LiveKit 本地配置（自动生成）
port: 7880
rtc:
  tcp_port: 7881
  port_range_start: 7882
  port_range_end: 7892
  use_external_ip: false
keys:
  ${lkKey}: $lkSecret
logging:
  level: info
  json: false
room:
  auto_create: true
"@ | Set-Content "$out\config\livekit.yaml" -Encoding UTF8 -NoNewline
Write-Host "  ok  .env + livekit.yaml（key=$lkKey）"

# ---------- [6/6] 复制运行时模板 ----------
Write-Host "`n[6/6] 复制启动器 + README..." -ForegroundColor Yellow
$tpl = Join-Path $PSScriptRoot 'templates'
Copy-Item "$tpl\start-kaede.ps1" $out -Force
Copy-Item "$tpl\start-kaede.bat" $out -Force
Copy-Item "$tpl\stop-kaede.bat" $out -Force

$readme = (Get-Content "$tpl\README.txt" -Raw) `
    -replace '\{\{VERSION\}\}', $Version `
    -replace '\{\{LIVEKIT_KEY\}\}', $lkKey `
    -replace '\{\{BUILD_TIME\}\}', (Get-Date -Format 'yyyy-MM-dd HH:mm')
$readme | Set-Content "$out\README.txt" -Encoding UTF8 -NoNewline
Write-Host "  ok  start-kaede.bat / stop-kaede.bat / start-kaede.ps1 / README.txt"

# ---------- 汇总 ----------
$total = ((Get-ChildItem $out -Recurse -File | Measure-Object Length -Sum).Sum)/1MB
$count = (Get-ChildItem $out -Recurse -File).Count
Write-Host "`n========================================" -ForegroundColor Green
Write-Host "打包完成: $out" -ForegroundColor Green
Write-Host ("  $count 文件  {0:N1} MB" -f $total)
Write-Host "========================================" -ForegroundColor Green
Get-ChildItem $out | Select-Object Name, @{n='Type';e={if($_.PSIsContainer){'dir'}else{'file'}}} | Format-Table -AutoSize
Write-Host "下一步: 打包 ZIP 发给内测者" -ForegroundColor Cyan
Write-Host "  Compress-Archive -Path '$out\*' -DestinationPath '$tag.zip'" -ForegroundColor DarkGray
