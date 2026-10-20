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
    cargo cbr         # build release-wasm

    # 开发
    dx serve --package kaede-wasm --platform web
    dx serve --package kaede-desktop --platform desktop

## 文档

- docs/KaedeNiwa v2 完整技术文档.txt    单一入口（整合版）
