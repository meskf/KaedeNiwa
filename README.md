# KaedeNiwa

**专为私有域设计的开源安全即时通讯系统。** 基于 Rust 全栈构建，以 IETF MLS（RFC 9420）为唯一密码学信任根，原生支持后量子密码学（Post-Quantum Ready），提供工业级的端到端加密与极简单节点部署体验。

- **服务端零信任（Zero-Knowledge Server）**：服务端仅作密文路由，不持有任何解密密钥。不依赖 `kaede-crypto`，真正做到物理隔离。
- **抗量子安全（Post-Quantum Ready）**：深度集成后量子密码套件（ML-KEM / X-Wing），防范“先存后解”（Harvest Now, Decrypt Later）攻击，保障数据长期机密性。
- **开箱即用（Self-Hosted）**：服务端编译为单一可执行文件（内嵌 Web UI 资源），数据落盘于本地 SQLite，零外部组件依赖，极低运维成本。

## 架构总览

**Rust Workspace (`crates/`)**
- `kaede-protocol` / 线协议类型（CBOR，协议 v8）
- `kaede-crypto` / AEAD / HKDF / SFrame / dm_cipher
- `kaede-mls` / 封装 openmls 0.9（支持 PQ hybrid ciphersuite）
- `kaede-storage` / 存储抽象（SQLite native / IndexedDB wasm）
- `kaede-client` / 客户端状态机（MLS / HTTP / WS / RoomSession）
- `kaede-server` / 信令服务器（axum 0.8 + WS + SQLite）
- `kaede-client-wasm` / WASM 桥接（JS <-> Rust）
- `kaede-denoise` / RNNoise 原生壳（未纳入 workspace，遗留）

**Frontend & Apps (`apps/`)**
- `web` / Web 客户端（React 19 + Vite 6 + Tailwind 4）
  - `e2e-standard/` / Playwright 标准验收套件
  - `e2e/` / 旧自制 e2e（run.cjs）
- `desktop` / Electron 壳（无边框 + 自绘 titlebar）

**Deploy (`deploy/`)**
- Caddy / Nginx / Docker / systemd scripts
- 构建、打包、备份脚本

## 密码学与安全模型

- **单一信任根**：MLS (RFC 9420) 是唯一的密码学信任根。频道 / DM / SFrame 媒体密钥 / LiveKit E2EE 全部从 MLS exporter 派生。
- **抗量子计算（Post-Quantum）**：MLS 使用 hybrid PQ ciphersuite `MLS_128_MLKEM768X25519_CHACHA20POLY1305_SHA384_MLDSA44` (ML-KEM-768 + X25519 / ChaCha20-Poly1305 / SHA-384 / ML-DSA-44)。TLS 层由 Caddy 提供 X25519MLKEM768。

详见 [SECURITY.md](SECURITY.md)。

---

## 快速开始
*(如果下方有其他的部署指南，保留在此处)*
