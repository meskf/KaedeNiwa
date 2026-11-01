# KaedeNiwa

**Discord 的使用体验 + Signal 同级的端到端加密 + 私有化部署。**

不是 Discord 克隆——服务端读不到消息内容的私有化 Discord。
不是 Signal——带频道/服务器/媒体能力的 Signal。
不是 SaaS——自己部署在自己的 VPS / 家庭服务器。

## 架构总览

crates/  (Rust workspace)
  kaede-protocol/     线协议类型（CBOR，协议 v8）
  kaede-crypto/       AEAD / HKDF / SFrame / dm_cipher
  kaede-mls/          openmls 0.9 封装（PQ hybrid ciphersuite）
  kaede-storage/      存储抽象（SQLite native / IndexedDB wasm）
  kaede-client/       客户端状态机（MLS / HTTP / WS / RoomSession）
  kaede-server/       signaling server（axum 0.8 + WS + SQLite）
  kaede-client-wasm/  WASM 桥（JS <-> Rust）
  kaede-denoise/      RNNoise 原生壳（未纳入 workspace，遗留）

apps/
  web/       Web 客户端（React 19 + Vite 6 + Tailwind 4）
             e2e-standard/  Playwright 标准验收套件
             e2e/           旧自制 e2e（run.cjs）
  desktop/   Electron 壳（无边框 + 自绘 titlebar）

deploy/      Caddy / Nginx / Docker / systemd
scripts/     构建、打包、备份脚本

加密栈：MLS (RFC 9420) 是唯一信任根。频道 / DM / SFrame 媒体密钥 /
LiveKit E2EE 全部从 MLS exporter 派生。kaede-server 不依赖 kaede-crypto，
只转发密文。

后量子：MLS 使用 hybrid PQ ciphersuite
MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44
（ML-KEM-768 + X25519 / ChaCha20-Poly1305 / SHA-384 / ML-DSA-44）。
TLS 层由 Caddy 提供 X25519MLKEM768。详见 SECURITY.md。

## 快速开始

### 环境

- Rust 1.99（见 rust-toolchain.toml）
- Node 22+
- wasm-pack（cargo install wasm-pack --locked）
- Linux 需要 libdbus-1-dev + pkg-config（keyring 依赖）

### 一键启动

    pwsh -File scripts/setup-dev.ps1      # 首次：装依赖
    pwsh -File start-all.ps1              # 起 LiveKit + 后端 + 前端
    pwsh -File stop-all.ps1

前端 https://localhost:5173 · 后端 http://localhost:18080 · LiveKit ws://localhost:7880

start-all.ps1 参数：-SkipTest（不跑旧 e2e）、-CleanDb（清空数据库）。

> 改动 kaede-client 后必须重建 wasm：cd apps/web && npm run build:wasm
> 否则前端加载旧 wasm，客户端侧修复全是假阳性。start-all.ps1 已自动重建。

### 手动启动

    $env:LIVEKIT_API_KEY='devkey'; $env:LIVEKIT_API_SECRET='secret'
    $env:LIVEKIT_WS_URL='ws://127.0.0.1:7880'
    cargo run -p kaede-server --features dev-mode

    cd apps/web; npm run dev

## 测试与验收

### Playwright 标准套件（权威）

    cd apps/web
    $env:KAEDE_HTTP='1'
    npx playwright test                    # 7 spec / 13 用例
    npx playwright test --ui               # 交互调试

playwright.config.ts 内置 webServer，自动拉起后端 + Vite。不需要预先 start-all.ps1。

覆盖：登录 / 双端消息 / MLS epoch 一致 / E2EE 可逆 / 频道隔离 / 自动拉人 /
好友 DM / 附件 / 表情反应 / 断线重连 / 移动端首屏 / 频道 CRUD / 全局搜索。

### Rust 测试

    cargo ctall       # workspace（排 client-wasm），含集成测试
    cargo ctnative    # kaede-client native 集成测试

> channel_b2_e2e / dm_b2_e2e / room_e2e 由 native feature 门控。
> 不加 --features native 时是空 crate，running 0 tests 静默通过。必须用 ctnative。

### 旧自制 e2e（保留兼容）

    cd apps/web
    npm run test:e2e
    npm run test:e2e:fast
    npm run test:e2e:pq

### 代码检查

    cargo ck        # check --workspace --all-targets
    cargo cf        # fmt --all
    cargo cc        # clippy -D warnings
    cargo cwasm     # check kaede-client-wasm on wasm32

### CI

.github/workflows/ci.yml 四个 job：

    fmt     cargo fmt --all -- --check
    check   cargo check 核心 6 crate --all-targets
    test    cargo test 两段：workspace（排 client/wasm）+ client --features native
    e2e     构建 wasm + 后端，跑 Playwright（webServer 自动启停）

## 项目结构细节

### 消息身份（v8.1）

消息主键是服务端分配的 id（messages.id，AUTOINCREMENT），不是时间戳。
ts 是服务端权威时间用于排序。reaction / edit / delete 均按 target_id 定位。

公开频道发送流程：

    encrypt -> save_snapshot -> POST /api/messages 拿 (id, ts)
            -> WS 广播(id, ts) -> 本地持久化

接收端从 AppMessageReceived.id / ChannelMessage.id 拿到同一个 id。

兼容：MlsAppMessage 的 id/ts、ReactionPayload.target_id 均带 serde(default)，
旧客户端缺省为 0，接收端回退按 ts 匹配。

### 加密路径分流

    场景                  加密                      传输
    公开频道消息          主群 MLS app message       WS 广播 + POST 持久化
    私密频道消息          频道 group key (0x03)      POST + 服务端定向 fanout
    DM                    per-conv AEAD v2 seal      POST /api/dm
    公开频道 reaction     主群 MLS                   WS 广播（不入历史）
    私密频道 reaction     频道 group key             WS ChannelReaction 定向
    附件                  频道/DM group key          POST /api/attachments
    媒体                  LiveKit E2EE (MLS exporter) LiveKit SFU
    快照                  DPAPI/Keychain/Secret Svc  本地存储

判断依据是频道是否私密（is_private_channel），不是本地是否有 channel group——
公开频道在 join 流程中也会持有 channel group，用后者判断会把合法主群消息
误当泄露丢弃。

## 已知限制

- 协议 v8，MIN_SUPPORTED_VERSION = 7。v8 新增消息 id；v7 新增 typing。
- 屏幕共享视频轨接收端渲染：当前 Open Issue。音频轨正常，视频轨收不到
  （本地双开也复现，与局域网/E2EE/分辨率无关）。详见 HANDOFF.md「当前 Open Issue」及 `docs/history/HANDOFF-archive-2026-11-02.md` PART 34。
- 原生移动端不含语音/屏幕共享：build-mobile.ps1 默认不编译 GStreamer/LiveKit
  原生栈。需要时用 -WebShell 内嵌网页版。
- 移动端快照加密：无 Android/iOS Keystore 绑定，使用与 Web 相同的公开信息
  派生 key（App 私有沙箱）。
- 快照加密：Windows 用 DPAPI；macOS/Linux 用系统钥匙串。钥匙串不可用时拒绝
  持久化并暂停发送，不退回公开信息派生 key。
- 本地搜索：只搜本地已解密的频道与 DM 历史，服务端只有密文。
- 媒体 seed TTL 8 小时：崩溃/关页不调 /leave，seed 最迟 8 小时后轮换。
  配置 LiveKit webhook 后由 participant_left / room_finished 立即回收。
- kaede-denoise 不在 workspace：RNNoise 原生壳，实际降噪走前端
  @shiguredo/rnnoise-wasm。
- tools/livekit/livekit-server.exe：55MB，.gitignore 忽略，需自行下载放入。
- Windows 并发编译：webrtc-sys / tao 多核并发偶发失败。
  解决：$env:CARGO_BUILD_JOBS = "2"。

## 部署

单二进制 + SQLite，TLS 由 Caddy 或 Nginx 终结。

    pwsh -File scripts/bundle.ps1 web           # -> dist/web/public
    cargo build --release -p kaede-server       # -> target/release/kaede-server

- deploy/README.md — 部署、可观测性、备份恢复、升级回滚 runbook
- deploy/Dockerfile · deploy/docker-compose.yml
- deploy/kaede-server.service — systemd
- deploy/Caddyfile · deploy/nginx.conf

运行时端点：/health（存活）· /ready（DB 就绪）· /metrics（Prometheus）。
KAEDE_LOG_FORMAT=json 输出结构化日志。启动时校验配置，非法值直接退出。

备份（SQLite WAL 必须在线备份，不能复制文件）：

    pwsh -File scripts/backup-db.ps1 -DbPath C:\kaede\kaede.db

### 管理后台

独立 listener（默认 127.0.0.1:18081），X-Admin-Token 认证，只管理元数据，
看不到消息明文。

    $env:KAEDE_ADMIN_ENABLED='1'
    $env:KAEDE_ADMIN_TOKEN='<随机长 token>'
    $env:KAEDE_ADMIN_BIND='127.0.0.1:18081'
    cargo run -p kaede-server

未设 token 时生成一次性 token 打印到 stdout。建议保持 127.0.0.1 绑定 + SSH 隧道。

## 文档

- SECURITY.md — 安全模型、威胁边界、PQC 状态
- HANDOFF.md — 当前有效的交接信息（铁律 / 架构 / 加密路径 / 踩坑 / Open Issue）
- docs/history/HANDOFF-archive-2026-11-02.md — 完整历史记录（1900+ 行，30+ PART）
- deploy/README.md — 部署运维 runbook

## 工作方式约定

文档与注释可能滞后或有误导，以代码为准。改动后请同步更新本 README 的
「已知限制」与相关章节。

