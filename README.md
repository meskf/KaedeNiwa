# KaedeNiwa

Rust 原生的 E2EE 实时协作平台。

## 目标

- 全 Rust workspace：signaling + crypto + media + UI + 业务
- Dioxus 跨平台：Web / Desktop (Wry) / Mobile
- MLS (RFC 9420) 单一化密钥体系，服务器零解密
- 实时媒体走 LiveKit 内建 E2EE；离线落盘走 SFrame（key 从 MLS exporter 派生）
- 8K60 / 4K120 直播管线（附加能力）

## 结构

    crates/
      kaede-protocol/       线协议类型（CBOR）
      kaede-crypto/         密码学原语（AEAD / HKDF / SFrame）
      kaede-mls/            openmls 封装 + snapshot cipher
      kaede-storage/        存储抽象（memory / SQLite / IndexedDB）
      kaede-client/         客户端状态机 + HTTP + WS + RoomSession
      kaede-server/         signaling server（axum + WS + SQLite）
      kaede-ui/             Dioxus 组件（Web / Desktop / 移动端响应式）
      kaede-wasm/           Web 入口（PWA）
      kaede-desktop/        Desktop 入口（Wry）
      kaede-mobile/         原生移动端入口（Dioxus mobile / Android 优先）
      kaede-admin/          管理后台桌面端（Dioxus）
      kaede-capture/        屏幕采集 + NVENC + 音频（GStreamer）
      kaede-media/          媒体层（SFrame / LiveKit E2EE）
      kaede-media-session/  LiveKit 原生媒体会话
      kaede-livekit-web/    Web 端 LiveKit JS 绑定

## 常用命令

    cargo ck          # check --workspace
    cargo ct          # test --workspace
    cargo cf          # fmt
    cargo cc          # clippy
    cargo cwasm       # check wasm target
    cargo cwd         # check wasm32 (whole workspace)
    cargo cbr         # build release-wasm

    # 开发
    dx serve --package kaede-wasm --platform web
    dx serve --package kaede-desktop --platform desktop

    # 移动端（需 JDK 17+ / Android SDK+NDK）
    pwsh -File scripts/build-mobile.ps1                 # 原生 UI（文字/DM/附件）
    pwsh -File scripts/build-mobile.ps1 -WebShell `      # 内嵌网页版（含语音/屏幕共享）
        -WebUrl https://kaede.example.com -Release

## 移动端（Android 优先）

前置：JDK 17+、Android SDK（Platform + NDK）、`cargo install dioxus-cli`（需与 Dioxus 0.7 匹配）。
iOS 只能在 macOS 上构建（`--platform ios`），当前仓库只保证 Android 路径。

```powershell
# 原生 UI：文字 / DM / 附件 / 私密频道（不含语音）
pwsh -File scripts/build-mobile.ps1

# 内嵌网页版：语音 + 屏幕共享观看（推荐内测使用）
pwsh -File scripts/build-mobile.ps1 -WebShell -Release `
    -WebUrl https://kaede.example.com
```

- 原生 UI 不编译 GStreamer / LiveKit 原生栈，因此没有语音；进入语音频道会提示。
- web-shell 会全屏加载指定网页版地址；Android 模拟器访问宿主机用 `http://10.0.2.2:18080`，
  真机/生产必须 HTTPS（Android 9+ 默认拒绝明文）。本地 http 调试需在生成的 manifest 里
  临时加 `android:usesCleartextTraffic="true"`，不要带进发布包。
- web-shell 的**屏幕共享观看不依赖系统权限**；麦克风/摄像头采集还需要宿主 WebView 实现
  `onPermissionRequest` 并申请 `RECORD_AUDIO` / `CAMERA`，当前未接入。
- 移动端本地数据（SQLite 快照）落在 App 私有可写目录（`KAEDE_HOME` → cwd → 系统临时目录）。

## 已知限制

- **协议版本 v6**：v5 新增好友变更实时事件；v6 新增私密频道表情反应
  （频道 group key 加密 + 服务端定向 fanout）。需要服务端与客户端同时升级；旧客户端会被拒绝。
- **Web 语音 E2EE** 依赖 `livekit-client@2.14.0` 的 e2ee worker；离线 PWA 需缓存该 worker 脚本。
- **媒体统一走 LiveKit HTTP token**；旧版 SDP/ICE/SfuToken 协议变体已移除。
- **画质档位（8K/4K/码率）**：native 传入编码器，Web 通过 `getDisplayMedia` 分辨率约束生效；
  共享中切换档位会自动重建共享。
- **快照加密**：Windows 用 DPAPI；macOS/Linux Desktop 用系统钥匙串（Keychain / Secret Service）
  保存随机 key；钥匙串不可用时拒绝持久化并暂停发送，不再退回公开信息派生 key。
  Linux 构建需要 `libdbus-1-dev` + `pkg-config`。
- **本地搜索**：只搜索本地已解密的频道历史与 DM 历史；服务端只有密文，无法全文搜索。
- **原生移动端暂不含语音/屏幕共享**：`kaede-mobile` 默认构建不编译 GStreamer/LiveKit
  原生媒体栈。需要语音与屏幕共享观看时使用 `-WebShell` 构建，内嵌网页版客户端。
- **移动端快照加密**：当前没有 Android/iOS 系统 Keystore 绑定，使用与 Web 相同的
  公开信息派生 key（App 私有沙箱），安全边界与 Web 端一致。
- **媒体 seed TTL 8 小时**：崩溃/关页不会调用 `/leave`，seed 最迟在 8 小时后轮换；
  配置 LiveKit webhook 后由 `participant_left` / `room_finished` 立即回收。
- **Windows 并发编译**：`webrtc-sys` 和 `tao` 在多核并发编译时偶发元数据竞争失败。
  解决：`$env:CARGO_BUILD_JOBS = "2"` 后重试，或直接重试一次。
  CI 中已设 `CARGO_BUILD_JOBS: "4"` 作为保守值。
- **`_webrtc/` 依赖**：`LK_CUSTOM_WEBRTC` 指向 `_webrtc/`（预编译 LiveKit C++ 库，约 430MB，`.gitignore` 忽略）。
  首次 clone 后需要重新生成或从备份复制，否则 `kaede-media` 构建会失败。
- **`tools/livekit/livekit-server.exe`**：55MB，`.gitignore` 忽略，用 `scripts/download-livekit.ps1` 下载。

## 部署与运维

生产 / Beta 部署使用单二进制 + SQLite，TLS 由 Caddy 或 Nginx 终结：

- [deploy/README.md](deploy/README.md) — 部署、可观测性、备份恢复、升级与回滚 runbook
- [deploy/Dockerfile](deploy/Dockerfile) · [deploy/docker-compose.yml](deploy/docker-compose.yml)
- [deploy/kaede-server.service](deploy/kaede-server.service)（systemd）
- [deploy/Caddyfile](deploy/Caddyfile) · [deploy/nginx.conf](deploy/nginx.conf)

运行时端点：`/health`（存活）、`/ready`（DB 就绪）、`/metrics`（Prometheus）。
启动时校验配置，非法值直接退出。结构化日志：`KAEDE_LOG_FORMAT=json`。

```powershell
pwsh -File scripts/bundle.ps1 web        # dist/web/public
cargo build --release -p kaede-server    # target/release/kaede-server
```

数据库备份（SQLite WAL 必须用在线备份，不能直接复制文件）：

```powershell
pwsh -File scripts/backup-db.ps1 -Db C:\kaede\kaede.db
```

## 管理后台

独立 listener（默认 `127.0.0.1:18081`），通过 `X-Admin-Token` 认证，只管理元数据，
看不到任何消息明文。

    # 启动服务端时启用
    KAEDE_ADMIN_ENABLED=1 \
    KAEDE_ADMIN_TOKEN=<随机长 token> \
    KAEDE_ADMIN_BIND=127.0.0.1:18081 \
    cargo run -p kaede-server

    # 启动桌面端后台
    cargo run -p kaede-admin

- 未设置 `KAEDE_ADMIN_TOKEN` 时会生成一次性 token 并打印到服务端 stdout（不会进入
  `/api/logs`）。
- 远程运维建议保持 `127.0.0.1` 绑定 + SSH 隧道，或在前方加 HTTPS 反向代理；
  后台登录页会提示明文 HTTP 风险。
- 后台支持：仪表盘、用户封禁/解封（含原因）、频道删除、日志查看、systemd 服务模式命令生成。

## 文档

- [SECURITY.md](SECURITY.md) — 安全策略与威胁模型
- [deploy/README.md](deploy/README.md) — 部署、可观测性、备份恢复与升级 runbook
- 审计报告与产品方案按版本归档在本机 `_docs_backup_*.zip`，不随仓库发布。
