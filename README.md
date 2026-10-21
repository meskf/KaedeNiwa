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
      kaede-ui/             Dioxus 组件（占位，重做中）
      kaede-wasm/           Web 入口
      kaede-desktop/        Desktop 入口（Wry）
      kaede-capture/        屏幕采集 + NVENC + 音频（GStreamer）
      kaede-media/          媒体会话层（SFrame / LiveKit E2EE）
      kaede-livekit-spike/  LiveKit API 验证（实验）

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

## 已知限制

- **协议版本 v5**：新增好友变更实时事件；私密频道实时消息 + DM/私密频道密文 v2
  （发送者/接收者绑定与重放保护）。需要服务端与客户端同时升级；旧客户端会被拒绝。
- **Web 语音 E2EE** 依赖 `livekit-client@2.14.0` 的 e2ee worker；离线 PWA 需缓存该 worker 脚本。
- **媒体统一走 LiveKit HTTP token**；旧版 SDP/ICE/SfuToken 协议变体已移除。
- **画质档位（8K/4K/码率）**：native 传入编码器，Web 通过 `getDisplayMedia` 分辨率约束生效；
  共享中切换档位会自动重建共享。
- **私密频道暂不支持表情反应**（会经主群广播泄露频道 id，已显式禁用）。
- **媒体 seed TTL 8 小时**：崩溃/关页不会调用 `/leave`，seed 最迟在 8 小时后轮换。
- **Windows 并发编译**：`webrtc-sys` 和 `tao` 在多核并发编译时偶发元数据竞争失败。
  解决：`$env:CARGO_BUILD_JOBS = "2"` 后重试，或直接重试一次。
  CI 中已设 `CARGO_BUILD_JOBS: "4"` 作为保守值。
- **`_webrtc/` 依赖**：`LK_CUSTOM_WEBRTC` 指向 `_webrtc/`（预编译 LiveKit C++ 库，约 430MB，`.gitignore` 忽略）。
  首次 clone 后需要重新生成或从备份复制，否则 `kaede-media` 构建会失败。
- **`tools/livekit/livekit-server.exe`**：55MB，`.gitignore` 忽略，用 `scripts/download-livekit.ps1` 下载。

## 文档

- [docs/README.md](docs/README.md) — 文档索引（审计 / 产品方案 / 测试清单）
- [SECURITY.md](SECURITY.md) — 安全策略
