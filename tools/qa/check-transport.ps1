# 部署链路自检（在你的“客户端网络”上运行，验证到服务器的媒体端口真的通）
#
# 为什么需要它：带宽够 ≠ 媒体通。屏幕共享掉帧/断续最常见的部署级原因是
# UDP 7882 被安全组/防火墙拦掉，浏览器悄悄回落到 TCP 7881：
# 能连上看得到画面，但会周期性卡顿、撕裂、断续。带宽测速完全看不出来。
#
# 用法：
#   pwsh -File tools/qa/check-transport.ps1 -Server media.example.com
#   pwsh -File tools/qa/check-transport.ps1 -Server 1.2.3.4 -UdpPort 7882 -TcpPort 7881
#
# 退出码：0 = 全部通过；1 = 存在阻断（脚本会打印修复建议）。

param(
  [Parameter(Mandatory = $true)][string]$Server,
  [int]$UdpPort = 7882,
  [int]$TcpPort = 7881,
  [int]$HttpsPort = 443,
  [int]$TimeoutMs = 2500
)

$ErrorActionPreference = 'Stop'
$fail = 0

function New-StunBindingRequest {
  $bytes = New-Object byte[] 20
  $bytes[0] = 0x00; $bytes[1] = 0x01      # Binding Request
  $bytes[2] = 0x00; $bytes[3] = 0x00      # length 0
  $bytes[4] = 0x21; $bytes[5] = 0x12      # magic cookie
  $bytes[6] = 0xA4; $bytes[7] = 0x42
  for ($i = 8; $i -lt 20; $i++) {
    $bytes[$i] = Get-Random -Minimum 0 -Maximum 256
  }
  return $bytes
}

Write-Host "== KaedeNiwa 部署链路自检 ==" -ForegroundColor Cyan
Write-Host "服务器: $Server"

# 1) HTTPS/信令可达
try {
  $tcp = New-Object System.Net.Sockets.TcpClient
  $ok = $tcp.ConnectAsync($Server, $HttpsPort).Wait($TimeoutMs)
  $tcp.Close()
  if ($ok) { Write-Host ("[PASS] TCP {0}:{1} 可连接（网页/信令）" -f $Server, $HttpsPort) -ForegroundColor Green }
  else { Write-Host ("[FAIL] TCP {0}:{1} 无法连接" -f $Server, $HttpsPort) -ForegroundColor Red; $fail++ }
} catch {
  Write-Host ("[FAIL] TCP {0}:{1} - {2}" -f $Server, $HttpsPort, $_.Exception.Message) -ForegroundColor Red; $fail++
}

# 2) TCP 媒体端口（UDP 不通时的回落路径）
try {
  $tcp = New-Object System.Net.Sockets.TcpClient
  $ok = $tcp.ConnectAsync($Server, $TcpPort).Wait($TimeoutMs)
  $tcp.Close()
  if ($ok) { Write-Host ("[INFO] TCP {0}:{1} 可连接（媒体回落可用，但会更容易卡顿）" -f $Server, $TcpPort) -ForegroundColor Yellow }
  else { Write-Host ("[WARN] TCP {0}:{1} 不可连接" -f $Server, $TcpPort) -ForegroundColor Yellow }
} catch {
  Write-Host ("[WARN] TCP {0}:{1} - {2}" -f $Server, $TcpPort, $_.Exception.Message) -ForegroundColor Yellow
}

# 3) UDP 媒体端口：发一个标准 STUN Binding Request，LiveKit 的 ICE 服务会回响应
$udpOk = $false
try {
  $udp = New-Object System.Net.Sockets.UdpClient
  $udp.Client.ReceiveTimeout = $TimeoutMs
  $udp.Connect($Server, $UdpPort)
  for ($i = 0; $i -lt 3 -and -not $udpOk; $i++) {
    $req = New-StunBindingRequest
    [void]$udp.Send($req, $req.Length)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    while ((Get-Date) -lt $deadline) {
      try {
        $remote = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
        $resp = $udp.Receive([ref]$remote)
        if ($resp -and $resp.Length -ge 8 -and $resp[4] -eq 0x21 -and $resp[5] -eq 0x12 -and $resp[6] -eq 0xA4 -and $resp[7] -eq 0x42) {
          $udpOk = $true
          break
        }
      } catch { break }
    }
  }
  $udp.Close()
} catch {
  Write-Host ("[INFO] UDP 探测异常: {0}" -f $_.Exception.Message) -ForegroundColor Yellow
}

if ($udpOk) {
  Write-Host ("[PASS] UDP {0}:{1} 有 STUN 响应：媒体可走 UDP 直连" -f $Server, $UdpPort) -ForegroundColor Green
} else {
  Write-Host ("[WARN] UDP {0}:{1} 未收到响应：LiveKit 不一定会应答裸 STUN，无法据此判定。" -f $Server, $UdpPort) -ForegroundColor Yellow
  Write-Host ("       请以 App 内屏幕共享画面右下角徽标为准：显示 UDP=直连正常；" ) -ForegroundColor Yellow
  Write-Host "       显示「TCP 回落」说明 UDP $UdpPort 被拦，需在安全组/防火墙放行（Docker 还需 -p ${UdpPort}:${UdpPort}/udp）。" -ForegroundColor Yellow
}

Write-Host ""
if ($fail -eq 0) {
  Write-Host "结论：服务器可达。媒体 UDP 是否直连请以 App 徽标 / check-transport 的 UDP 结果为准。" -ForegroundColor Green
  exit 0
} else {
  Write-Host "结论：存在阻断项，请先按上面的提示修复，否则共享一定卡。" -ForegroundColor Red
  exit 1
}
