param(
    [ValidateSet('L0','L1','L2','L3','L4','L5','L6','L7','all')]
    [string[]]$Layer = @('L0'),
    [int]$Seed = 42,
    [int]$SoakMinutes = 10,
    [int]$SoakClients = 10,
    [switch]$ReuseEnv,
    [switch]$Force
)
$ErrorActionPreference = 'Continue'
# 本地 QA 编排：run-all 自己负责启动/复用 webServer。
# 若继承宿主 CI=true，Playwright 会强制 reuseExistingServer=false 并因端口已占用失败。
Remove-Item Env:CI -ErrorAction SilentlyContinue
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$QaDir = "$Root\tools\qa"
$RunDir = "$QaDir\.run"
$ReportDir = "$RunDir\reports"
New-Item -ItemType Directory -Force -Path $RunDir, $ReportDir | Out-Null

if ($Layer -contains 'all') { $Layer = @('L0','L1','L2','L3') }

$results = [System.Collections.Generic.List[object]]::new()

function Run-Layer([string]$name, [scriptblock]$body) {
    Write-Host ""
    Write-Host ("======== {0} ========" -f $name) -ForegroundColor Cyan
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        & $body
        if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { throw ("exit {0}" -f $LASTEXITCODE) }
        $sw.Stop()
        $secs = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        $results.Add([pscustomobject]@{ layer=$name; status='PASS'; duration=$secs; detail='' })
        Write-Host ("  PASS {0} ({1}s)" -f $name, $secs) -ForegroundColor Green
    } catch {
        $sw.Stop()
        $secs = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        $msg  = $_.Exception.Message
        $results.Add([pscustomobject]@{ layer=$name; status='FAIL'; duration=$secs; detail=$msg })
        Write-Host ("  FAIL {0} ({1}s): {2}" -f $name, $secs, $msg) -ForegroundColor Red
    }
}

foreach ($l in $Layer) {
    switch ($l) {
        'L0' { Run-Layer 'L0-protocol-crypto-mls-storage' {
            Push-Location $Root
            try {
                cargo test -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage --all-targets 2>&1 |
                    Tee-Object -FilePath "$ReportDir\L0.log" | Out-Host
            } finally { Pop-Location }
        } }
        'L1' { Run-Layer 'L1-server-integration' {
            Push-Location $Root
            try {
                cargo test -p kaede-server --all-targets 2>&1 |
                    Tee-Object -FilePath "$ReportDir\L1.log" | Out-Host
            } finally { Pop-Location }
        } }
        'L2' { Run-Layer 'L2-client-native' {
            Push-Location $Root
            try {
                $env:KAEDE_TEST_KEYSTORE = 'memory'
                cargo test -p kaede-client --features native --all-targets 2>&1 |
                    Tee-Object -FilePath "$ReportDir\L2.log" | Out-Host
            } finally { Pop-Location }
        } }
        'L3' { Run-Layer 'L3-playwright-standard' {
            $startedHere = -not $ReuseEnv
            if ($startedHere) { & "$QaDir\start-env.ps1" -Force | Out-Null }
            try {
                Push-Location "$Root\apps\web"
                try {
                    $env:KAEDE_HTTP = '1'
                    $env:KAEDE_HEADLESS = '1'
                    & "$Root\apps\web\node_modules\.bin\playwright.cmd" test --reporter=list 2>&1 |
                        Tee-Object -FilePath "$ReportDir\L3.log" | Out-Host
                } finally { Pop-Location }
            } finally {
                if ($startedHere) { & "$QaDir\stop-env.ps1" | Out-Null }
            }
        } }
        'L4' { Run-Layer 'L4-playwright-media' {
            $startedHere = -not $ReuseEnv
            if ($startedHere) { & "$QaDir\start-env.ps1" -WithMedia -Force | Out-Null }
            try {
                # L4 必须强制重启 vite（避免 HMR 残留模块，导致 import('/src/lib/livekit.ts')
                # 拿到的 currentRoom 与页面实际使用的实例不一致）
                if ($startedHere) {
                    $statePath = "$RunDir\state.json"
                    if (Test-Path $statePath) {
                        $state = Get-Content $statePath -Raw | ConvertFrom-Json
                        if ($state.web.pid) { Stop-Process -Id $state.web.pid -Force -EA SilentlyContinue }
                        Get-NetTCPConnection -LocalPort $state.web.port -State Listen -EA SilentlyContinue |
                            ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -EA SilentlyContinue }
                        Start-Sleep -Milliseconds 800
                        & "$QaDir\start-env.ps1" -WithMedia | Out-Null
                    }
                }
                Push-Location "$Root\apps\web"
                try {
                    $env:KAEDE_HTTP = '1'
                    $env:KAEDE_HEADED = '1'
                    & "$Root\apps\web\node_modules\.bin\playwright.cmd" test --config=../../tools/qa/playwright/playwright.l4.config.ts --reporter=list 2>&1 |
                        Tee-Object -FilePath "$ReportDir\L4.log" | Out-Host
                } finally { Pop-Location }
            } finally {
                if ($startedHere) { & "$QaDir\stop-env.ps1" | Out-Null }
            }
        } }
        'L5' { Run-Layer 'L5-soak' {
            $startedHere = -not $ReuseEnv
            if ($startedHere) { & "$QaDir\start-env.ps1" -WithMedia -Force | Out-Null }
            try {
                $soakMin  = if ($PSBoundParameters.ContainsKey('SoakMinutes')) { $SoakMinutes } else { 10 }
                $soakCli  = if ($PSBoundParameters.ContainsKey('SoakClients')) { $SoakClients } else { 10 }
                # L5 同样强制重启 vite（防 HMR 残留）
                if ($startedHere) {
                    $statePath = "$RunDir\state.json"
                    if (Test-Path $statePath) {
                        $state = Get-Content $statePath -Raw | ConvertFrom-Json
                        if ($state.web.pid) { Stop-Process -Id $state.web.pid -Force -EA SilentlyContinue }
                        Get-NetTCPConnection -LocalPort $state.web.port -State Listen -EA SilentlyContinue |
                            ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -EA SilentlyContinue }
                        Start-Sleep -Milliseconds 800
                        & "$QaDir\start-env.ps1" -WithMedia | Out-Null
                    }
                }
                node "$QaDir\soak.mjs" --clients $soakCli --minutes $soakMin --seed $Seed 2>&1 |
                    Tee-Object -FilePath "$ReportDir\L5.log" | Out-Host
            } finally {
                if ($startedHere) { & "$QaDir\stop-env.ps1" | Out-Null }
            }
        } }
        'L6' { Run-Layer 'L6-human-multi-e2e' {
            $startedHere = -not $ReuseEnv
            # 必须用全新 QA 库：每个场景都会创建独立频道，旧库里残留的 general
            # 群权威会让新用户触发 takeover，影响结果可复现性。
            if ($startedHere) { & "$QaDir\start-env.ps1" -Force | Out-Null }
            try {
                Push-Location "$Root\apps\web"
                try {
                    $env:KAEDE_HTTP = '1'
                    $env:KAEDE_HEADLESS = '1'
                    & "$Root\apps\web\node_modules\.bin\playwright.cmd" test `
                        --config=playwright.multi.config.ts --reporter=list 2>&1 |
                        Tee-Object -FilePath "$ReportDir\L6.log" | Out-Host
                } finally { Pop-Location }
            } finally {
                if ($startedHere) { & "$QaDir\stop-env.ps1" | Out-Null }
            }
        } }
        'L7' { Run-Layer 'L7-server-restart-chaos' {
            $startedHere = -not $ReuseEnv
            # 重启混沌必须从干净库开始，否则 general 群的旧权威会干扰重连验证。
            if ($startedHere) { & "$QaDir\start-env.ps1" -Force | Out-Null }
            try {
                # 脚本内部会 kill 后端 → 等 10s → start-env（保留 DB，重启后端）
                node "$QaDir\chaos-restart.mjs" --clients 4 --minutes 2 --restartAt 30 2>&1 |
                    Tee-Object -FilePath "$ReportDir\L7.log" | Out-Host
            } finally {
                if ($startedHere) { & "$QaDir\stop-env.ps1" | Out-Null }
            }
        } }
    }
}

$summary = [pscustomobject]@{
    seed      = $Seed
    startedAt = (Get-Date).ToString('o')
    results   = $results
    passed    = ($results | Where-Object status -eq 'PASS').Count
    failed    = ($results | Where-Object status -eq 'FAIL').Count
}
$summary | ConvertTo-Json -Depth 6 | Set-Content "$ReportDir\summary.json" -Encoding UTF8

Write-Host ""
Write-Host "======== Summary ========" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-Host
Write-Host ("报告: {0}" -f "$ReportDir\summary.json") -ForegroundColor DarkGray
if ($summary.failed -gt 0) { exit 1 }
exit 0
