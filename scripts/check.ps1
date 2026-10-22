# 全量检查
Set-Location $PSScriptRoot\..
# webrtc-sys / tao 在 Windows 多核并发编译时偶发 MSVC C1060（编译器堆空间不足）
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = "2" }
cargo check --workspace --all-targets