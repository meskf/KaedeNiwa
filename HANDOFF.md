# KaedeNiwa HANDOFF

最后更新：2026-11-02
面向：接手的工程师（人或 AI）

> 完整历史交接记录（1900+ 行，30+ 个 PART，含大量已解决的踩坑细节）已归档至
> `docs/history/HANDOFF-archive-2026-11-02.md`。
> 本文件只保留**当前仍然有效**的信息。需要考古时查归档。

## 必读：三条铁律

1. **读代码 > 读注释与文档。** 注释和文档经常滞后或错误。有冲突时以代码为准。
2. **改动 kaede-client 后必须重建 wasm。** `cd apps/web && npm run build:wasm`。
   否则前端加载旧 `kaede_client_wasm_bg.wasm`，所有客户端侧修复都是假阳性。
   诊断线索：`apps/web/src/wasm/pkg/*.wasm` 的 mtime < `crates/` 下源文件 mtime。
3. **`kaede-client` 的 native 集成测试由 feature 门控。**
   `channel_b2_e2e` / `dm_b2_e2e` / `room_e2e` 文件顶部是 `#![cfg(feature = "native")]`。
   不加 `--features native` 时整个文件编译成空 crate，`cargo test` 显示
   `running 0 tests` 并**静默通过**。必须用 `cargo ctnative`。

## 架构速查

    协议层    kaede-protocol   CBOR 线协议，v8（MIN_SUPPORTED=7）
    密码学    kaede-crypto     AEAD / HKDF / SFrame / dm_cipher
    群组      kaede-mls        openmls 0.9，PQ hybrid ciphersuite
    存储      kaede-storage    SQLite (native) / IndexedDB (wasm)
    客户端    kaede-client     状态机 + MLS session + HTTP + WS + RoomSession
    服务端    kaede-server     axum 0.8，不依赖 kaede-crypto，纯盲转发
    WASM 桥   kaede-client-wasm  薄封装，60+ #[wasm_bindgen] 导出
    前端      apps/web         React 19 + Vite 6 + Tailwind 4
    壳        apps/desktop     Electron（无边框 + 自绘 titlebar）

## 加密路径（改动前必看）

    公开频道消息     主群 MLS app message        WS 广播 + POST 持久化
    私密频道消息     频道 group key (0x03)       POST + 服务端定向 fanout
    DM               per-conv AEAD v2 seal       POST /api/dm
    公开频道 reaction 主群 MLS                   WS 广播（不入历史）
    私密频道 reaction 频道 group key             WS ChannelReaction 定向
    附件             频道/DM group key           POST /api/attachments
    媒体             LiveKit E2EE (MLS exporter) LiveKit SFU

**判断分流依据是频道是否私密（`is_private_channel`），不是本地是否有 channel
group。** 公开频道在 join 流程中也会持有 channel group；用 has_group 判断会把
合法的公开频道消息误当「私密内容经主群泄露」丢弃。这是 2026-11-02 修过的真实 bug。

## 消息身份（v8.1，2026-11-02 落地）

消息主键是服务端 `messages.id`（AUTOINCREMENT），不是时间戳。
reaction / edit / delete 全部按 `target_id` 定位。

发送顺序**必须**是：

    encrypt -> save_snapshot -> POST 拿 (id, ts) -> WS 广播(id, ts) -> 本地持久化

先广播后 POST 会导致接收端拿到的 ts 是 0，UI 退化成 `Date.now()`，
与 history 的服务端 ts 永远不相等，reaction 挂不上目标消息。

## 踩坑清单（仍然相关）

### wasm 缓存
改 `kaede-client` 后不重建 wasm，前端跑的是旧代码。修复一度全是假阳性。
现在 `start-all.ps1` 和 `run-all-tests.ps1` 都自动重建，但手动 `cargo run` +
`npm run dev` 时不会——记得手动 `npm run build:wasm`。

### 门控测试静默通过
见「三条铁律 3」。`cargo test --workspace` 显示 0 tests 不等于测试通过。
CI 已拆成两段（workspace + client native）避免漏跑。

### Playwright webServer 与 cargo profile
`playwright.config.ts` 的 webServer 用 `cargo run`（debug）。
CI 的构建步骤必须也用 debug（`cargo build --features dev-mode`），
否则会重复编译 release，e2e job 白等几分钟。

### nodes 表与 migrations
`nodes` 表 2026-11-02 前只在 `db::init_nodes_table()` 内联创建，不在 migrations。
任何绕过 `main.rs` 初始化顺序的路径（`spawn_test_server`、`ws::mk_state`）
都会缺表 → `no such table: nodes` → HTTP 500。
现已纳入 `migrations/20261030000001_nodes.sql`，测试辅助仍显式调 `init_nodes_table`
做老 channels → nodes 的幂等数据同步。

### 单次解密
MLS 密文只能解密一次（ratchet 前进后不可重放）。`inbound.rs` 曾对同一密文
调用 `mls_decrypt` + `decrypt_with_sender` 两次，第二次必失败，
导致 `AppMessage` 永不 emit，且 sender 校验 / 私密频道过滤 / MediaMeta 识别
全部落在死路径上。现在单次解密拿 `(plaintext, mls_sender)`，安全逻辑在活路径。

### 频道分流判据
见「加密路径」最后一段。`is_private_channel` vs `channel_has_group` 的区别
踩过一次，改动分流逻辑时务必区分。

### 前端事件接口
`ClientEvent` 的 serde 序列化是 externally tagged（`{"AppMessageReceived": {...}}`）。
前端 `MainView.tsx` 按这个形状解构。新增事件类型时前端需要同步加分支，
否则事件被静默忽略。

## 当前 Open Issue

**屏幕共享视频轨接收端渲染**（HANDOFF 归档 PART 34 有完整调试记录）

- 现象：发送端正常，接收端收到音频轨并解密播放，但收不到视频轨。
- 已排除：局域网、防火墙、WebRTC 穿透、E2EE（`?noe2ee=1` 也复现）、
  分辨率、Simulcast。**电脑本地双开也复现。**
- 调试痕迹：`TrackPublished` / `TrackSubscribed` / `TrackSubscriptionFailed`
  监听器都加过，强制 `setSubscribed(true)` 无效。SDP 里接收端 Offer 有时
  甚至没有 `m=video` 行。
- 下一步：独立最小 WebRTC 页（`getDisplayMedia` + `RTCPeerConnection`，不依赖项目代码）
  验证原生链路；抓 LiveKit `--dev` 模式 SFU 日志确认是否收到推流端视频数据。

## 测试基线（2026-11-02）

    cargo ctall          217 passed / 0 failed / 8 ignored
    cargo ctnative       112 passed / 0 failed
    npx playwright test  13 passed / 0 failed（7 spec）
    cargo cc             4 个既有 warning，非本轮引入

## 工作方式建议

- 改动前先跑一遍基线，确认起点是绿的。
- 修 bug 优先找**根因**，不要给症状打补丁。本轮 reaction 失败的根因是
  「发送先广播后 POST」而非「ReactionBar 渲染逻辑」，只改渲染会永远修不好。
- 大改动拆成可验证的小步，每步跑一次相关测试。本轮 id 贯通拆了 7 轮。
- 不确定的引用关系先搜索再删。本轮清洁前扫了 `run.cjs` 的 `file:` 字段、
  `test-helpers.cjs` 的引用者、根目录 cjs 的全仓引用，确认孤立后才移动。
- 破坏性操作（删除、移动、协议变更）先做可逆版本（移到 `_archive/`、
  加 `serde(default)`），确认无副作用后再考虑彻底清理。
