# KaedeNiwa 状态快照

最后更新：2026-11-02

## 当前版本

- 协议 v8（MIN_SUPPORTED_VERSION = 7），workspace 版本 0.1.0
- 8 个 crate，7 个在 workspace（kaede-denoise 未纳入）
- 前端：apps/web（React 19 + Vite 6 + Tailwind 4）+ apps/desktop（Electron）

## 测试基线（2026-11-02 实测全绿）

    Rust workspace   217 passed / 0 failed / 8 ignored
    Rust client native 112 passed / 0 failed
    Playwright e2e-standard  13 passed / 0 failed（7 spec）
    clippy           4 个既有 warning（metrics/channels/nodes/ws 旧代码）

## 最近完成（2026-11-02）

1. CI 补强：新增 test job，两段跑 workspace + client --features native，
   消灭「测试存在但 CI 从不执行」盲区。
2. Playwright webServer：本地与 CI 共用同一份启动/探活逻辑，
   替代原先手写 spawn/wait 和交互式 start-all.ps1。
3. 消息身份 v8.1：从 ts 迁移到服务端 id，贯通协议/服务端/客户端/wasm/前端。
   修复 reaction 永远挂不上对端消息的缺陷（Playwright 05 此前一直失败）。
4. 安全修复：inbound.rs 双解密导致 sender 校验与私密频道过滤落在死路径，
   实际生效路径不校验发送者——已合并为单次解密 + 活路径校验。
5. 功能修复：notify_authority 成功入队仍返回 false（离线权威加入频道误报 409）。
6. 功能修复：begin_media_session 无条件走 channel key，DM 媒体 key 永远拿不到。
7. 架构修复：nodes 表纳入 migrations，统一生产与测试初始化路径。
8. 仓库清洁：222 个临时脚本/日志/备份移入 _archive/2026-11-cleanup
   （含 MANIFEST.json 逐条记录，可逆）。根目录只剩规范文件。
9. 文档重写：README（架构/快速开始/测试/已知限制）、STATUS（本文件）、
   .cargo/config.toml 别名修正。

## 待办（按优先级）

### P1
- 屏幕共享视频轨接收端渲染（HANDOFF PART 34 Open Issue）
- safety number / 密钥指纹核验（Signal 同级定义性特征，计划正式版前做）
- 元数据最小化：presence / typing 目前服务器可见

### P2
- wasm/src/lib.rs（30KB，60+ 导出）拆模块 + TS 类型化
- HANDOFF.md 拆分归档（现 1950 行多会话累积）
- Playwright 扩 firefox/webkit smoke 子集

### P3
- kaede-crypto / kaede-mls 挂 cargo-fuzz 持续跑
- kaede-denoise 决定纳入 workspace 或标记 deprecated

## 已知 Open Issue

**屏幕共享视频轨**：发送端正常、接收端收不到视频轨（音频轨正常）。
已排除局域网、防火墙、E2EE、分辨率、Simulcast。本地双开复现。
下一步：独立 WebRTC 最小页验证 + 抓 LiveKit SFU 日志。
详见 HANDOFF.md PART 34。

