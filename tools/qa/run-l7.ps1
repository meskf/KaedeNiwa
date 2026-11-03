$ErrorActionPreference = 'Continue'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $Root
$ReportDir = Join-Path $Root 'tools\qa\.run\reports'
New-Item -ItemType Directory -Force -Path $ReportDir | Out-Null
$log = Join-Path $ReportDir 'L7.log'
$json = Join-Path $ReportDir 'L7.json'
Remove-Item $json, $log -Force -ErrorAction SilentlyContinue

& pwsh -NoProfile -File (Join-Path $Root 'tools\qa\stop-env.ps1') | Out-Null
& pwsh -NoProfile -File (Join-Path $Root 'tools\qa\start-env.ps1') -Force | Out-Null

# start-env 探活成功后 vite 可能仍在重新优化依赖（新 wasm pkg），端口会短暂断开。
# 等连续 3 次 200 再开始，避免客户端首屏 ERR_CONNECTION_REFUSED。
$stable = 0
$waited = 0
for ($i = 0; $i -lt 60; $i++) {
    try {
        $r = Invoke-WebRequest 'http://127.0.0.1:5173/' -UseBasicParsing -TimeoutSec 2
        if ($r.StatusCode -eq 200) { $stable++ } else { $stable = 0 }
    } catch { $stable = 0 }
    if ($stable -ge 3) { break }
    $waited++
    Start-Sleep -Seconds 1
}
"# vite stable after ${waited}s" | Out-File -FilePath $log -Encoding UTF8

$env:KAEDE_TEST_MODE = '1'
node (Join-Path $Root 'tools\qa\chaos-restart.mjs') --clients 4 --minutes 2 --restartAt 30 *>> $log
$code = $LASTEXITCODE
"EXIT=$code" | Out-File -FilePath $log -Append -Encoding UTF8

& pwsh -NoProfile -File (Join-Path $Root 'tools\qa\stop-env.ps1') | Out-Null
exit $code
