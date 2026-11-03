KaedeNiwa 独立服务端（Windows）
================================

开箱即用：
  1. 双击 start-kaede.cmd
  2. 浏览器会自动打开 http://127.0.0.1:18080
  3. 首个注册的账号自动成为服主（op5），可创建频道/文件夹/私密频道
  4. 之后注册的账号默认是访客（op1），只能聊天，需服主在“成员”里提升权限
  5. 首次启动 Windows 防火墙弹窗时选择“允许访问”（局域网/语音需要）

目录说明：
  kaede-server.exe   聊天/E2EE 服务端
  livekit-server.exe 语音/视频媒体服务
  web/               Web 客户端静态文件（服务端自带）
  livekit.yaml       LiveKit 配置（含媒体密钥）
  data/              运行时数据（首次启动自动创建）
  logs/              运行日志

局域网访问：
  - 其他设备打开 http://<本机 IP>:18080
  - 需要放行 TCP 18080（网页/接口）、TCP 7880（语音信令）、
    TCP 7881 + UDP 7882（语音媒体）
  - Android 客户端在登录页“服务器地址”填 http://<本机 IP>:18080

修改媒体密钥：
  同时修改 livekit.yaml 的 keys 与 start-kaede.ps1 的 LIVEKIT_API_SECRET，
  两处必须完全一致，否则语音会鉴权失败。

停止：
  双击 stop-kaede.cmd

注意：
  - 当前二进制未做代码签名，Windows SmartScreen 可能提示“未知发布者”，
    选择“更多信息 → 仍要运行”即可。
  - 生产环境建议放在 Caddy/Nginx 之后并启用 HTTPS；Android 客户端已允许明文
    HTTP 以便内网直连。
