# 验证 KaedeNiwa 部署的 TLS 1.3 是否协商到 X25519MLKEM768（hybrid PQ）
#
# 用法：
#   pwsh -File verify-pq-tls.ps1 -Domain kaede.example.com
#
# 前提：
#   - 目标域名已部署 Caddy 2.10+（默认启用 x25519mlkem768）
#   - 本机有 Docker（用 debian:trixie-slim 自带 OpenSSL 3.5）
#
# 说明：
#   Caddy 不记录协商曲线（Go tls.ConnectionState 不暴露），
#   因此验证必须从客户端侧完成。

param(
    [Parameter(Mandatory = $true)]
    [string]$Domain,

    [int]$Port = 443
)

$ErrorActionPreference = 'Stop'

Write-Host "=== PQC TLS verification: $Domain`:$Port ===" -ForegroundColor Cyan
Write-Host "  probing with OpenSSL 3.5 (debian:trixie-slim) ..."
Write-Host ""

$cmd = @"
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq openssl >/dev/null 2>&1
echo | openssl s_client -connect $Domain`:$Port -groups X25519MLKEM768 -brief 2>&1 \
  | grep -E 'Protocol version|Negotiated TLS1.3 group|Ciphers|error|alert'
"@

$output = docker run --rm debian:trixie-slim sh -c $cmd 2>&1 | Out-String
Write-Host $output

if ($output -match 'Negotiated TLS1\.3 group:\s*X25519MLKEM768') {
    Write-Host "  PASS: PQ hybrid group negotiated" -ForegroundColor Green
    exit 0
} elseif ($output -match 'Negotiated TLS1\.3 group:\s*(x25519|secp|X25519)') {
    Write-Host "  PARTIAL: TLS 1.3 OK but classical curve negotiated" -ForegroundColor Yellow
    Write-Host "           check that Caddy >= 2.10 and Caddyfile has curves x25519mlkem768"
    exit 2
} else {
    Write-Host "  FAIL: could not verify (see output above)" -ForegroundColor Red
    exit 1
}
