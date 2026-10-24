# 停止所有 KaedeNiwa 服务
Write-Host "停止 KaedeNiwa 服务..." -ForegroundColor Cyan
$ports = @(18080, 5173, 7880, 7881, 7882)
$count = 0
foreach ($p in $ports) {
    $conns = Get-NetTCPConnection -LocalPort $p -State Listen -ErrorAction SilentlyContinue
    foreach ($c in $conns) {
        $procId = $c.OwningProcess
        $procName = (Get-Process -Id $procId -ErrorAction SilentlyContinue).Name
        if ($procName) {
            Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
            Write-Host "  ✅ 停止 $procName (端口 $p, PID $procId)" -ForegroundColor Green
            $count++
        }
    }
}
if ($count -eq 0) {
    Write-Host "  无运行中的服务" -ForegroundColor DarkGray
}
Write-Host "完成" -ForegroundColor Cyan
