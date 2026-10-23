# KaedeNiwa 移动端构建（Android 优先）
#
# 原生 UI（文字 / DM / 附件）：
#   pwsh -File scripts/build-mobile.ps1
#
# 内嵌网页版壳（含语音 / 屏幕共享观看）：
#   pwsh -File scripts/build-mobile.ps1 -WebShell -WebUrl https://kaede.example.com
#
# 前置：JDK 17+、Android SDK + NDK、`cargo install dioxus-cli`（版本需与 0.7 匹配）。
param(
    [switch]$WebShell,
    [switch]$Release,
    [string]$WebUrl
)

$ErrorActionPreference = 'Stop'

if (-not (Get-Command dx -ErrorAction SilentlyContinue)) {
    Write-Host '错误: 未找到 dx（dioxus-cli）。请先 cargo install dioxus-cli。' -ForegroundColor Red
    exit 1
}
if (-not $env:ANDROID_HOME -and -not $env:ANDROID_SDK_ROOT) {
    Write-Host '错误: 未设置 ANDROID_HOME / ANDROID_SDK_ROOT。' -ForegroundColor Red
    Write-Host '  Android Studio → SDK Manager 安装 Platform + NDK，然后设置环境变量。' -ForegroundColor Yellow
    exit 1
}
if (-not $env:JAVA_HOME -and -not (Get-Command java -ErrorAction SilentlyContinue)) {
    Write-Host '错误: 未找到 JDK。请安装 JDK 17+ 并设置 JAVA_HOME。' -ForegroundColor Red
    exit 1
}

$dxArgs = @('build', '--package', 'kaede-mobile', '--platform', 'android')
if ($Release) { $dxArgs += '--release' }
if ($WebShell) {
    $dxArgs += @('--features', 'web-shell')
    if ($WebUrl) {
        $env:KAEDE_MOBILE_WEB_URL = $WebUrl
        Write-Host "web-shell 地址: $WebUrl" -ForegroundColor Cyan
    } else {
        Write-Host '未指定 -WebUrl，使用编译期默认 http://10.0.2.2:18080（仅模拟器开发）' -ForegroundColor Yellow
    }
}

& dx @dxArgs
if ($LASTEXITCODE -ne 0) {
    Write-Host "构建失败 (exit $LASTEXITCODE)" -ForegroundColor Red
    exit $LASTEXITCODE
}
Write-Host '构建完成。' -ForegroundColor Green
