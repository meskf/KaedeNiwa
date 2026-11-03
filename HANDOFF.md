# KaedeNiwa HANDOFF

最后更新：2026-11-03
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

## 2026-11-03 本轮改动（务必了解）

### 发送/落库
- wasm `send_message` / `sendMessageWithAttachments` 现在返回服务端权威
  `{"id":..,"ts":..}`，前端不再插入无 id 的乐观消息。
  `chat.ts` 的 `setMessages` 是**原子合并**（在 zustand set 回调里读当前列表），
  不要改回“外部先读 existing 再整体替换”——会和发送并发丢消息。

### 频道群接管
- 新增 `POST /api/channels/{id}/takeover`：仅当该频道**其他成员全部离线**时允许；
  服务端清空其他成员，并给每人排 `channel_removed` 信封。
- 客户端 join 遇到 409 `no online authority`：先等 8s Welcome；超时才请求 takeover。
  超时且频道成员数 >1 时也走 takeover（服务端在线成员检查会兜底）。
- 被移除成员的客户端收到 `ChannelRemoved` → `ChannelDeleted` 事件；
  MainView 会丢弃旧 group，并在仍停留在该频道时自动重新加入。
- **不要**在“有在线权威”时自建群，否则会永久分裂。

### 会话/登出
- `signOut()`（apps/web/src/wasm/client.ts）= forceDisconnect + POST /api/logout + 清 auth。
- 服务端 login 发新 token 时会主动断开该账号旧 WS（否则旧页面变僵尸）。
- 客户端处理 `ServerError` 的 snake_case 码（unauthenticated/kicked/
  unsupported_version）立即回登录页；重连耗尽也会 signOut。

### 心跳 / 断线语义
- wasm 心跳：10s ping / 20s timeout / 2.5s 检查。旧实现 20s tick + 30s 超时，
  实际检测最坏 ~60s。
- **传输层必须区分主动关闭与意外断开**：`WebSocketTransport` 带 `closing` 标记，
  用户主动 `close()` 才返回 `ClientError::Closed`；服务端崩溃、网络断开、
  `onerror`、远端正常关闭都返回 `Transport` 错误，让 event loop emit
  `Disconnected`。否则 event loop 会把 `Closed` 当作正常退出、置 `shutdown`，
  心跳随之停止，客户端永久“看似在线”但消息发不出（L7 曾因此失败）。
  native 与 wasm 两套 transport 都要保持该语义。

### 测试基建
- `tools/qa/`：L0–L7；QA 独立库 `tools/qa/.run/qa.db`，`-Force` 重建。
- `start-env.ps1` 注入 `KAEDE_TEST_MODE=1`、`KAEDE_DB`、admin（:18081）。
- e2e-multi 每个场景用 `setupIsolatedChannel` 自建频道；L6 默认串行。
  跨场景并行会 churn 主群/频道群权威，属测试 artifact。
- `run-all.ps1` 会清除继承的 `CI`；否则 Playwright webServer 强制
  `reuseExistingServer=false`，与 QA env 已占用端口冲突。

## 测试基线（2026-11-03）

    cargo ctall          全绿（0 failed / 8 ignored）
    cargo ctnative       112 passed / 0 failed
    npx playwright test  14 passed / 0 failed（e2e-standard）
    e2e-multi            10 spec / 21 passed / 1 skipped 串行全绿
    L7 chaos-restart     4 clients，重启 16s，4/4 重连，交叉可见 PASS，0 pageError
    L5 soak（8 clients, mixed）  18/18 共享、66/66 消息，阈值全过
    cargo cc             0 warning

## 当前 Open Issue

**主群 host 生命周期**（P0 架构级）

reaction / media 信令走主群 MLS，而主群仍以“第一个用户 = host”为中心。
host 离线后新用户可能各自建主群，跨主群 reaction/media 信令不可解。
频道消息走独立频道群，已有 takeover 兜底，不受此影响。
下一步：主群也需要 generation/takeover，或改为每频道信令。

（屏幕共享视频轨已在 2026-11-03 修复：LiveKit JS 2.23 在 screen_share 上同时
启用 simulcast + screenShareSimulcastLayers 会静默丢弃 video track；改为单层编码后
L5 media soak 18/18 共享成功。）

## 工作方式建议

- 改动前先跑一遍基线，确认起点是绿的。
- 修 bug 优先找**根因**，不要给症状打补丁。本轮 reaction 失败的根因是
  「发送先广播后 POST」而非「ReactionBar 渲染逻辑」，只改渲染会永远修不好。
- 大改动拆成可验证的小步，每步跑一次相关测试。本轮 id 贯通拆了 7 轮。
- 不确定的引用关系先搜索再删。本轮清洁前扫了 `run.cjs` 的 `file:` 字段、
  `test-helpers.cjs` 的引用者、根目录 cjs 的全仓引用，确认孤立后才移动。
- 破坏性操作（删除、移动、协议变更）先做可逆版本（移到 `_archive/`、
  加 `serde(default)`），确认无副作用后再考虑彻底清理。



## 打包 / 发布（2026-11-04，版本 0.1.3）

发布地址：https://github.com/meskf/KaedeNiwa/releases/tag/v0.1.3（0.1.2 同流程，已发布）。
产物与校验在 `release/`：`README-0.1.3.md`、`SHA256SUMS.txt`、`notes-0.1.3.md`。
0.1.3 只修桌面端全新安装的默认服务器地址（`LoginView` 检测 `window.electronAPI`），
外加版本号统一，服务端功能未变。
- Windows 独立包：`tools/package-server.ps1`（依赖 `target/release/kaede-server.exe`、
  `tools/livekit/livekit-server.exe`、`apps/web/dist`、`deploy/standalone/*`）。
  打完后解压到干净目录、`start-kaede.ps1` 启动、跑 `deploy-smoke.spec.ts` 复核。
- Docker：仓库根 `docker-compose.yml`（本机/局域网试跑，8080 无 TLS）与
  `deploy/docker-compose.yml`（域名 + Caddy 自动证书的生产编排）。
- 桌面：`apps/desktop` 下 `npm run build:client`（electron-builder 打 NSIS + portable，
  把 `../web/dist` 作为 extraResources 打进 `resources/web`）。
- Android：`apps/web` 下 `npx cap sync android` -> `android/gradlew.bat assembleRelease`
  -> `zipalign -p -f 4` -> `apksigner sign --ks-key-alias kaede`
  （keystore `release/keys/kaede-release.jks`，密码 `kaede-release`）。
  版本号在 `android/app/build.gradle` 的 versionCode/versionName，需与 workspace 版本同步。

改前端后三端都要重打：dist 变了 -> APK 与桌面包里的前端就过期了（对比 mtime 即可发现）。
`deploy-smoke.spec.ts` 要求服务端是“全新实例”（首个注册账号才是 op5 服主），
每轮验收前先清空数据（Docker: `docker compose down -v`；独立包: 清 `data/`）。

浏览器 + `http://<局域网IP>` 下 Web Crypto 不可用（非安全上下文），前端会显示提示页；
跨机浏览器访问必须 HTTPS。桌面端（本地 127.0.0.1 源）与 Android（https://localhost 源）不受影响。