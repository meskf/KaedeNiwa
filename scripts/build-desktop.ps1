# 构建 Desktop
Set-Location $PSScriptRoot\..\crates\kaede-desktop
# webrtc-sys 在 Windows 多核并发编译时偶发 MSVC C1060（编译器堆空间不足）
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = "2" }
dx build --platform desktop --release