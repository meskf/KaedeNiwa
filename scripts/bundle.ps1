# KaedeNiwa 打包脚本
# 用法: .\scripts\bundle.ps1 web    # 打包 Web
#       .\scripts\bundle.ps1 desktop # 打包 Desktop

param(
    [Parameter(Mandatory=$true)]
    [ValidateSet("web", "desktop")]
    [string]$Platform
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

if (-not (Get-Command dx -ErrorAction SilentlyContinue)) {
    Write-Host "错误: 未找到 dx 命令。请先安装 dioxus-cli:" -ForegroundColor Red
    Write-Host "  cargo install dioxus-cli --locked" -ForegroundColor Yellow
    exit 1
}

switch ($Platform) {
    "web" {
        Write-Host "打包 Web (release-wasm)..." -ForegroundColor Cyan
        Push-Location crates\kaede-wasm
        dx bundle --platform web --release --out-dir ..\..\dist\web
        Pop-Location
        Write-Host "`n输出: dist\web" -ForegroundColor Green
    }
    "desktop" {
        Write-Host "打包 Desktop (Wry)..." -ForegroundColor Cyan
        Push-Location crates\kaede-desktop
        dx bundle --platform desktop --release --out-dir ..\..\dist\desktop
        Pop-Location
        Write-Host "`n输出: dist\desktop" -ForegroundColor Green
    }
}
