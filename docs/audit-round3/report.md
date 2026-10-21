# KaedeNiwa 第三轮全量代码审查与缺陷报告

- 审查日期：2026-10-21
- 审查对象：当前工作区全部 13 个 crate（约 3.3 万行 Rust，含 tests / examples），不是 `HEAD`
- 审查方式：5 路并行只读审查 + 关键结论逐行复核 + 上游 SDK（`livekit-client@2.14.0`、`livekit-0.9.5`）源码交叉验证
- 修复状态：2026-10-21 **P0×2 / P1×16 / P2×24 已全部修复**；fmt / check / clippy / 228 tests / wasm 全量回归通过（见第 7 节）

---

## 0. 项目理解（审查基线）

KaedeNiwa 是“服务器零解密”的 E2EE 实时协作平台，核心信任边界如下：

- **服务器权威**：账号/会话、presence、消息落库与 `ts`、频道成员关系、离线信封、LiveKit token；但不应能解密消息内容。
- **客户端权威**：MLS 身份与 epoch、消息加解密、本地快照、设备密钥。

三条聊天链路：

1. **公开频道**：客户端 `mls_send_app` → WS `MlsAppMessage` 全服广播；HTTP 只负责历史落库。
2. **私密频道**：频道独立 MLS group → exporter key → `dm_cipher::seal`（首字节 `0x03`）→ HTTP 落库 + 服务端 `ChannelMessage` 定向推送成员。
3. **DM**：双方 MLS group → `dm_derive_key` → `dm_cipher`（首字节 `0x01`）→ HTTP 落库 + 定向投递。

身份持久化：MLS provider 快照（主群 + 每个 DM/频道各一份 aux 快照）经 `SnapshotCipher` 加密后写入 SQLite / IndexedDB。

媒体：服务端签发 LiveKit token + 房间级 `e2ee_seed`；客户端用本地 group exporter 与 seed 派生 E2EE key。原生走 Rust LiveKit SDK（PBKDF2），Web 走 `livekit-client` UMD（`ExternalE2EEKeyProvider`）。

UI：Dioxus 0.7，Web（wasm）/ Desktop（wry）；`observer` 把 `ClientEvent` 应用到 `AppState`；M1–M4 新增移动端布局。

### 本轮基线（实测）

| 项目 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | ✅ 通过 |
| `cargo clippy -p <核心 6 crate> --all-targets -- -D warnings` | ✅ 通过 |
| `cargo test -p <核心 6 crate>` | ✅ 228 passed / 0 failed / 3 ignored |
| `cargo check -p kaede-wasm --target wasm32-unknown-unknown` | ✅ 通过 |
| `cargo fmt --all -- --check` | ✅ 通过（修复后） |

结论（审计时）：构建/测试基线良好，但存在 2 个 P0（Web 端身份持久化、Web 端语音 E2EE 实际未启用）和一批会破坏功能/安全边界的问题。**上述问题已在同一轮修复完成，当前回归全部通过。**

---

## 1. P0 — 必须修复

### P0-1 Web 端快照“写用 fallback、读用 DEK” → 浏览器每次刷新都丢 MLS 身份

**位置**

- 写入：[room.rs:596-618](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L596-L618) 使用 [`device_cipher`](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/keystore.rs#L240-L250)；非 Windows（含 wasm）恒为 `SHA256(uid,url)` 派生 key。
- 读取：[room.rs:162-166](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L162-L166)、[room.rs:292-293](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L292-L293) 使用 [`device_cipher_for_snapshot`](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/keystore.rs#L258-L297)；wasm 上 `ALG_AES_GCM` 分支优先用 `cached_dek()`。
- DEK 初始化：[room.rs:102-106](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L102-L106)、[room.rs:261-265](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L261-L265)。

**机制**：浏览器正常登录时 `init_web_keystore()` 成功 → `CACHED_DEK=Some`。第一次写快照用 fallback key（`alg_id=ALG_AES_GCM`）；刷新后恢复时，wasm 分支看到同一个 `alg_id` 却用 DEK 解密 → GCM 校验失败 → `restore failed, starting fresh` → `mls_init` 生成新身份。房主还会重新 `become_host()` 重建主群；DM/频道 aux 快照同样走这套读写（[connection.rs:884](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/connection.rs#L884)、[connection.rs:1056](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/connection.rs#L1056)、[connection.rs:1114](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/connection.rs#L1114)），因此私聊/私密频道也会一起失效。

**影响**：Web 端每次刷新相当于换了一个设备身份；旧历史不可解、其他成员被 Welcome 拉进新群、DM 可能永久分裂。这与 [keystore.rs:252-256](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/keystore.rs#L252-L256) 注释“AES-GCM 必须用 fallback cipher 才能解”直接矛盾。

**修复建议**：让 `device_cipher` 在 wasm 且 `cached_dek()` 存在时返回 DEK cipher（与读取端一致）；或给快照写入明确的 key-id/版本，读写共用同一分派函数。补一个 wasm 侧“写→读”往返测试。

---

### P0-2 Web 端语音 E2EE 实际从未启用（SDK 字段名错误 + 缺 worker）

**位置**

- [wasm_impl.rs:57-79](file:///D:/dev2/kaedeniwa/crates/kaede-livekit-web/src/wasm_impl.rs#L57-L79)：把 `keyProvider` 塞进 `opts.encryption`。
- [lib.rs:23](file:///D:/dev2/kaedeniwa/crates/kaede-livekit-web/src/lib.rs#L23)、[index.html:16](file:///D:/dev2/kaedeniwa/crates/kaede-wasm/index.html#L16)：锁定 `livekit-client@2.14.0`。
- Web 语音已真实接线：[voice_web.rs:42-59](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/voice_web.rs#L42-L59)，侧栏 wasm 分支调用 `connect_voice`。
- [media-session/session.rs:110-112](file:///D:/dev2/kaedeniwa/crates/kaede-media-session/src/session.rs#L110-L112)：原生侧 key 缺失时只 `warn` 后继续明文连接。

**证据（上游源码）**：`livekit-client@2.14.0` 的 `RoomOptions` 只有 `e2ee?: E2EEOptions`，不存在 `encryption`；`E2EEOptions` 为 `{ keyProvider: BaseKeyProvider, worker: Worker }`。因此 `opts.encryption` 被静默忽略，`E2eeManager` 不会创建，浏览器直接以 DTLS-SRTP（SFU 可解密）发布，而日志仍打印 `e2ee=true`（只检查了 keyProvider 是否存在）。

**影响**：Web 用户以为在端到端加密通话，实际媒体在 SFU 明文可解；与 native 端 C14 承诺不一致。README 的“Web 语音不可用”已过时（Web 语音现已接线）。

**修复建议**：改为 `opts.e2ee = { keyProvider, worker }`，worker 使用与 SDK 同版本的 e2ee worker 脚本；E2EE 初始化失败必须返回错误或在 UI 显式警告，禁止静默降级。

---

## 2. P1 — 高优先级

### P1-1 私密频道本地无 group 时会先向主群广播明文

**位置**：[room.rs:1233-1245](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L1233-L1245)（注释写明“不能退回主群”，代码却在 `else` 分支 `mls_send_app`）、[room.rs:1295-1307](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L1295-L1307)、附件 [room.rs:1157-1167](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L1157-L1167)。

**机制**：判断条件只有 `channel_has_group()`。当服务端已把用户写入 `channel_members`（离线权威加入路径）但本地 `join_channel` 超时/快照恢复失败时，本地没有频道 group；此时消息先经 `MLS` 主群 key 加密并由 WS 全服广播，HTTP 之后才被 [http.rs:492-499](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/http.rs#L492-L499) 以 400 拒绝。附件更糟：`upload_attachment` 不校验频道，主群 key 加密的附件会成功落盘。

**影响**：私密频道文本/附件密钥对全体主群成员可用（可解密），私密性被静默破坏；UI 只显示“发送失败”。

**修复建议**：客户端缓存频道 `is_private` 集合；私密频道且无 channel group 时直接返回错误，禁止进入主群分支；附件上传同样拒绝。

### P1-2 并发 `host_add_peer` 会用错误 epoch 标记 Commit → 永久群分裂

**位置**：[room.rs:422-466](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L422-L466)（`mls_add_member` 后先 `await` 发送 Welcome，再读取**当时**的 epoch 发 Commit：[room.rs:516-522](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L516-L522)）。

**机制**：两个 `PeerJoined` 并发时，A 的 add 产生 epoch N+1，随后在 await Welcome 期间 B 的 add 推进到 N+2；A 恢复后用 N+2 标记自己的 N+1 Commit。服务端按 epoch 主键只保留第一份 Commit（[ws.rs:693-715](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L693-L715)），接收方再按 epoch 排序补投，顺序必然错乱，最终不同成员的 epoch 不一致。

**修复建议**：所有 MLS 变更（add/remove/self_update）串行化（`tokio::sync::Mutex`），并在生成 Commit 的同一临界区返回/捕获其 epoch；广播使用生成时的 epoch，不要事后读取。

### P1-3 恢复态生成的 KeyPackage 私钥不落盘 → 服务端 KP 队列全成死包

**位置**：[connection.rs:382-388](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/connection.rs#L382-L388) 生成 KP 未 `mark_main_dirty`；[room.rs:390-393](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L390-L393) 随后 `save_snapshot()`；[room.rs:600-603](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L600-L603) 在 dirty=false 时直接返回。

**触发**：快照恢复成功但还没有主群（guest 未被 Add），`restored=true`、`dirty=false`；`become_guest` 生成并上传 20 个 KP，但快照没写。重启后这 20 个 KP 的私钥不存在，房主逐个拉取全部失败，guest 长期无法入群。

**修复建议**：`mls_generate_key_package` 后 `mark_main_dirty()`（或在 `publish_key_packages` 成功后强制落盘一次），并对上传成功的 KP 记录“已持久化”状态。

### P1-4 快照落盘失败只 `warn`，继续发送 → 重启后 ratchet 回退 / nonce 复用

**位置**：[room.rs:461-464](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L461-L464)、[room.rs:1246-1249](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L1246-L1249)、[room.rs:1308-1311](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L1308-L1311)。

**影响**：IndexedDB 配额满、磁盘满、只读 DB 等持续失败时，内存 ratchet 前进而磁盘 generation 停在旧值；重启后正是第一轮审计 P0-1 的 MLS AES-GCM nonce 复用场景，且用户无感知。

**修复建议**：连续失败 N 次（或首次失败）即发出 `MlsSyncError` 并阻止继续发送，直到保存成功；UI 显式提示。

### P1-5 附件 ID 含 `-` 时永久 400（约 29% 上传不可下载）

**位置**：[attachments.rs:96-97](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/attachments.rs#L96-L97) 用 `URL_SAFE_NO_PAD`（字母表含 `-`、`_`）取前 22 字符；[attachments.rs:140-144](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/attachments.rs#L140-L144) 只允许字母数字与 `_`。

**影响**：`1-(63/64)^22 ≈ 29%` 的附件 ID 含 `-`，文件已落盘但 `GET /api/attachments/{id}` 永远 400。属功能性回归。

**修复建议**：白名单加入 `'-'`（仍然排除 `/`、`\`、`.`），或改为十六进制 ID。

### P1-6 生产入口叠加第二个 CORS 层 → 重复 `Access-Control-Allow-Origin`

**位置**：[lib.rs:38-66](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/lib.rs#L38-L66) 已按 `KAEDE_CORS_ORIGINS` 配置 CORS；[main.rs:66-71](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/main.rs#L66-L71) 又加一层 `allow_origin(Any)`。

**影响**：白名单来源会收到两条 ACAO（具体 origin 与 `*`），浏览器按规范拒绝；`KAEDE_CORS_ORIGINS` 配置实际失效，默认本地开发端口可能全部 CORS 失败，Web 端无法连接独立部署的服务器。

**修复建议**：删除 `main.rs` 的第二个 CORS 层，统一由 `with_middleware` 决定。

### P1-7 被踢/被删账号的 WS 在 ≤5 分钟内可重新 Join，且服务端不主动关闭

**位置**：[members.rs:342-359](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/members.rs#L342-L359) 仅 `take_peer` + `try_send(Kicked)`，不关闭 socket；[ws.rs:440-486](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L440-L486) 的 `Join` 只信任 Hello 时缓存的 `my_uid`，不重查 `sessions`；[ws.rs:246-256](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L246-L256) 才是 5 分钟一次的 token 重验。

**影响**：删除/吊销账号后，旧连接可立刻重新 Join，最长 5 分钟继续接收 DM、全服广播并可发送消息；单设备接管也只是“建议性”踢人。

**修复建议**：`Join` 时重新校验 session token（并检查账号仍存在）；踢人时强制关闭旧 socket（drop sender 或关闭底层连接），而不是只发一条错误消息。

### P1-8 离线信封可被任意已 Join 用户塞满（16 MiB × 100 / 受害者）

**位置**：[ws.rs:653-676](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L653-L676) `MlsWelcome` 不校验目标存在/关系、不限 payload；[db.rs:54-91](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/db.rs#L54-L91) 每用户保留 100 条但无字节上限；[db.rs:99-132](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/db.rs#L99-L132) 一次性全量取出；[ws.rs:392-397](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L392-L397) 再塞进 1024 槽队列。

**影响**：单个攻击者向离线受害者发 100 条 ~16 MiB（协议上限）Welcome，受害者 Join 时服务端需要 1.6 GiB 内存/带宽；同时新条目按 id 挤掉合法 Welcome，构成定向 E2EE 拒绝服务。`ChannelSync` 同样只校验发送者是频道成员，不校验收件人。

**修复建议**：信封 payload 上限（如 256 KiB）+ 每用户总字节配额；`MlsWelcome` 校验目标账号存在且（若适用）同频道；投递成功后再删除（见 P2）。

### P1-9 `mls_commits.epoch` 可被网络输入永久污染

**位置**：[ws.rs:693-715](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L693-L715) 直接 `epoch as i64` 入库；[ws.rs:719-736](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L719-L736) 按 epoch 补投。

**触发**：已 Join 用户发送 `MlsCommit { epoch: i64::MAX as u64 }`。`MAX(epoch)` 永久变为 `i64::MAX`，随后 `DELETE WHERE epoch <= MAX-256` 会清空所有合法 commit；`ReportEpoch` 还会把伪造 commit 补投给客户端。

**影响**：离线 epoch 追赶功能全局失效，并可注入伪 Commit；单账号即可破坏全服主群同步。

**修复建议**：校验 epoch 为合理 `u32`/`u64` 范围并限制单调增长；拒绝与当前状态差异过大的 epoch；commit 需绑定发送者在组内（服务端无法验证时至少限制在合理窗口）。

### P1-10 解密历史明文跨账号共享，本地自己的消息也跨账号串号

**位置**：[room.rs:720-722](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L720-L722) 缓存前缀只有 `server_url|channel`；[room.rs:752-765](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L752-L765) 缓存的是明文；[room.rs:629-643](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L629-L643)、[room.rs:671-702](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L671-L702) 本地消息前缀无 uid，且读取时 sender 被强制写成当前 uid；UI 所有账号共用一个 `kaedeniwa.db`（[login.rs:217](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/login.rs#L217)）。

**影响**：同一浏览器/机器换账号后，新账号能读到上一账号解密过的频道历史明文；上一账号自己发的消息还会被显示成新账号“自己发的”。共享设备上属隐私泄漏。

**修复建议**：存储 namespace/前缀加入 uid（或每账号独立 DB）；历史缓存至少用设备密钥加密；本地消息读取使用缓存中的真实 sender。

### P1-11 移动端“私聊/好友”两个 Tab 永远为空

**位置**：[mobile_pages.rs:62-113](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/mobile_pages.rs#L62-L113)、[mobile_pages.rs:115-163](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/mobile_pages.rs#L115-L163) 只读 `app.dm_conversations` / `app.friends`；唯一加载器在桌面组件内：[dm_view.rs:85](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/dm_view.rs#L85)、[dm_view.rs:361](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/dm_view.rs#L361)、[friends_view.rs:11](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/friends_view.rs#L11)、[friends_view.rs:36](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/friends_view.rs#L36)；[mobile_shell.rs:45-49](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/mobile_shell.rs#L45-L49) 不挂载这两个组件。

**影响**：移动端用户即使已有会话/好友，也只会看到“暂无私聊/暂无好友”，且没有进入成员选择器的入口。

**修复建议**：在 `MobileDmPage`/`MobileFriendsPage` 各加 `use_effect` 调用现成加载函数，或登录后统一预取。

### P1-12 Web 与 native 的 LiveKit E2EE KDF 不一致；导出失败还会静默降级

**位置**：Web [wasm_impl.rs:63-70](file:///D:/dev2/kaedeniwa/crates/kaede-livekit-web/src/wasm_impl.rs#L63-L70) 传 `Uint8Array`；native [media-session/session.rs:98-108](file:///D:/dev2/kaedeniwa/crates/kaede-media-session/src/session.rs#L98-L108) 显式 `KeyDerivationAlgorithm::PBKDF2`。

**证据（上游源码）**：`livekit-client@2.14.0` 的 `BaseKeyProvider.setKey`：字符串走 PBKDF2，`ArrayBuffer` 走 HKDF；Rust SDK 默认/显式都是 PBKDF2。即使修好 P0-2 的字段名，Web↔native 仍会派生出不同 key，表现为互通黑屏/无声。

**降级路径**：[room.rs:1679-1706](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L1679-L1706) 在 group 存在但 exporter 失败时返回 `Ok((token, None))`；[media-session/session.rs:110-112](file:///D:/dev2/kaedeniwa/crates/kaede-media-session/src/session.rs#L110-L112) 只 `warn` 后继续明文；[voice_web.rs:47-59](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/voice_web.rs#L47-L59) 同样接受 `None`。

**修复建议**：统一两端的 KDF 与输入编码（例如统一 hex 字符串 + PBKDF2）；导出失败时返回可区分错误/事件，由 UI 阻断或显式警告，不能静默明文。

### P1-13 原生 `unpublish` 不真正撤销 track，远端永远收不到 TrackGone

**位置**：[session.rs:31-36](file:///D:/dev2/kaedeniwa/crates/kaede-media-session/src/session.rs#L31-L36) 的 `sid` 是字面量 `"screen"`/`"audio-…"` 且标注“保留供未来用”；[session.rs:533-547](file:///D:/dev2/kaedeniwa/crates/kaede-media-session/src/session.rs#L533-L547) 只停喂帧任务，全仓没有 `unpublish_track`。

**影响**：关屏/关麦后再开，远端一直显示“正在共享/麦克风开”，并叠加多条空 track；`publish` 也可能因 `published` 表残留而拒绝重发。

**修复建议**：保存真实 `publication.sid()`，`unpublish` 时调用 `local_participant().unpublish_track(&sid)`；发布任务异常退出时同样清理并发送 `LocalUnpublished`。

### P1-14 发布任务自行退出后不清理 `published`，会话永久不可重发

**位置**：[session.rs:301-369](file:///D:/dev2/kaedeniwa/crates/kaede-media-session/src/session.rs#L301-L369)：3 秒无帧/通道关闭直接 `break`，既不 `remove` map entry，也不 unpublish、不发事件；表项只在 [session.rs:534](file:///D:/dev2/kaedeniwa/crates/kaede-media-session/src/session.rs#L534) 的 `unpublish` 中删除。

**影响**：硬件不支持该档位、设备被占用、编码器崩溃后，track 永久黑屏/静音，后续 `publish` 返回 `already published`，只能离开频道重进。

**修复建议**：任务出口统一做 `remove + unpublish_track + LocalUnpublished/Error`，并用 guard 兜底。

### P1-15 4K/8K 采集被静默压成 1080p，SDP 与实际分辨率不一致

**位置**：[encoded.rs:283-295](file:///D:/dev2/kaedeniwa/crates/kaede-capture/src/encoded.rs#L283-L295) `width/height=None` 时默认 1920×1080 并写入 GPU capsfilter；[encoded.rs:352-356](file:///D:/dev2/kaedeniwa/crates/kaede-capture/src/encoded.rs#L352-L356) 用同一 fallback 选择编码器；[encoded.rs:323-328](file:///D:/dev2/kaedeniwa/crates/kaede-capture/src/encoded.rs#L323-L328) CPU 路径不缩放但 SDP 仍通告 1080p。

**影响**：默认（不显式指定分辨率）时 4K/8K 屏幕被缩到 1080p；8K 仍按 1080p 选 H.264 → 编码器失败/3 秒无帧。与 `tests/video_config.rs` “default must keep native resolution”的承诺矛盾。

**修复建议**：`None` 时读取源 caps 的实际分辨率，再决定 scale/码率/SDP；或显式要求 UI 传入分辨率。

### P1-16 DM/私密频道密文不绑定发送者、方向与序号（服务器可反射/重放）

**位置**：[dm_cipher.rs:59-86](file:///D:/dev2/kaedeniwa/crates/kaede-crypto/src/dm_cipher.rs#L59-L86) AAD 仅 `conv_id`；收发双方共用同一 key；接收端直接信任服务器 `from`（[connection.rs:1539-1576](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/connection.rs#L1539-L1576)）。

**影响**：恶意/失陷服务器可把 A→B 的密文反射回 A 并标为 `from: B`，A 解密成功并显示为 B 的消息；同一密文可无限重放。与 `SECURITY.md` “服务器不可伪造合法消息”的威胁模型不符。

**修复建议**：AAD 加入 `sender || recipient || direction || counter(u64 BE)`，客户端维护 per-conversation 单调序号窗口拒重放。

---

## 3. P2 — 健壮性 / 数据一致性 / 体验

| # | 问题 | 位置 | 影响 / 建议 |
|---|---|---|---|
| P2-1 | `cargo fmt --check` 失败 | [sidebar.rs:1026-1030](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/sidebar.rs#L1026-L1030) | CI 阻断；`cargo fmt --all` 即可 |
| P2-2 | 协议解码不限制集合元素数 | [protocol/lib.rs:621-626](file:///D:/dev2/kaedeniwa/crates/kaede-protocol/src/lib.rs#L621-L626)、`PublishKeyPackages.packages` [lib.rs:176-178](file:///D:/dev2/kaedeniwa/crates/kaede-protocol/src/lib.rs#L176-L178)、`Joined.peers` [lib.rs:417-419](file:///D:/dev2/kaedeniwa/crates/kaede-protocol/src/lib.rs#L417-L419) | 16 MiB 的“空 byte string 数组”可产生 ~400 MB 分配；需探针/上限验证。建议 serde 计数上限 |
| P2-3 | 私密频道改名无成员校验 | [channels.rs:209-273](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/channels.rs#L209-L273) | 曾加入后退出者仍可改名并广播；应要求频道成员或 op5 |
| P2-4 | session token 哈希可直接当 token | [http.rs:366-373](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/http.rs#L366-L373)、[ws.rs:791-802](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L791-L802) | `token = hash(?) OR token = ?` 使 DB 泄漏的哈希可直接认证；移除明文兼容分支 |
| P2-5 | 离线信封先删后投 | [db.rs:99-132](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/db.rs#L99-L132)、[ws.rs:392-397](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/ws.rs#L392-L397) | 投递失败即永久丢失；应 ack 后删除或失败回滚 |
| P2-6 | 昵称双写不一致 | [http.rs:180](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/http.rs#L180)、[http.rs:223-227](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/http.rs#L223-L227)、[members.rs:179-216](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/members.rs#L179-L216) | 改昵称后重新登录/`/api/me` 仍是旧值；PATCH 同步 accounts.nickname |
| P2-7 | `update_member` 不校验控制字符 | [members.rs:100-115](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/members.rs#L100-L115) | 昵称/bio 可含换行/ANSI，污染广播/日志；复用注册校验 |
| P2-8 | 历史加载整体替换，覆盖在途实时消息 | [content.rs:130-157](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/content.rs#L130-L157)、[dm_view.rs:8-37](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/dm_view.rs#L8-L37) | 请求返回后 `insert` 覆盖，实时消息从 UI 消失；应合并去重 |
| P2-9 | 加入私密频道弹窗状态不复位 | [sidebar.rs:782-793](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/sidebar.rs#L782-L793)、[sidebar.rs:909-951](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/sidebar.rs#L909-L951)、[sidebar.rs:314-320](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/sidebar.rs#L314-L320) | 再次打开会锁在旧频道 ID；双击文字频道也可能沿用 `then_voice` 自动开麦 |
| P2-10 | 自己消息的实时表情永远不显示 | [content.rs:565](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/content.rs#L565)、[payload.rs:37](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/payload.rs#L37)、[observer.rs:629](file:///D:/dev2/kaede-ui/src/observer.rs#L629) | 乐观插入 ts 与 payload ts 是两次 `now_ms()`，reaction 精确匹配失败；建议引入稳定 message id |
| P2-11 | “加载更早”后被强制滚到底部 | [content.rs:101-106](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/content.rs#L101-L106) | effect 依赖列表长度，任何增长都 `scrollTop=scrollHeight`；应只在尾部增长且接近底部时滚动 |
| P2-12 | 同作者连发时回复引用不显示 | [content.rs:288-302](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/content.rs#L288-L302) | reply 块放在 `if !merged` 内；移到外部 |
| P2-13 | Toast 定时器互相踩踏 | [app.rs:120-137](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/app.rs#L120-L137) | 旧任务会把新提示提前清掉；清除前校验 token |
| P2-14 | DM 显示时间与排序/去重时间源不同 | [observer.rs:238-251](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/observer.rs#L238-L251) | `ts` 用 `p.ts`，`day/time` 用服务器 `ts`；翻页/排序可能错位，建议统一服务器 ts |
| P2-15 | 附件内联预览信任发送方 `size` | [attachment.rs:57-59](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/attachment.rs#L57-L59)、[attachment.rs:84-88](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/attachment.rs#L84-L88) | 声明 `size=0` 的 32 MiB 图片会生成 43 MB data URL；以 `bytes.len()` 为准 |
| P2-16 | 移动端 Tab 双高亮、语音频道按文字频道打开 | [mobile_shell.rs:121-124](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/mobile_shell.rs#L121-L124)、[mobile_pages.rs:43-49](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/mobile_pages.rs#L43-L49) | “个人”映射到 Friends；语音频道无加入语音分支，可发文字 |
| P2-17 | 平板布局成员列表按钮无效 | [shell.rs:181](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/shell.rs#L181)、[shell.rs:201-205](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/shell.rs#L201-L205) | `members_open` 恒被 `!compact` 置 false，点击后再被置 false；应隐藏或允许浮层 |
| P2-18 | `fetch_kp_with_retry` 对不可重试错误也重试 | [room.rs:472-501](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L472-L501) | 每次重试都消费服务端一个一次性 KP；只应对 404/网络错误重试 |
| P2-19 | wasm WS 连接无超时 | [transport.rs:216-295](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/transport.rs#L216-L295) | 注释称“最多 10s”，实际只等 open/error；不可达主机可能卡数分钟 |
| P2-20 | DM 媒体 token 接受虚构会话 | [livekit.rs:259-268](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/livekit.rs#L259-L268)、[dm.rs:51-55](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/dm.rs#L51-L55) | `dm:<自己>:<随机>` 通过字符串校验，可无限创建 seed/participants；应查 `dm_conversations` 并限流 |
| P2-21 | 无认证请求先缓冲 32–34 MiB body | [lib.rs:65](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/lib.rs#L65)、[attachments.rs:65-80](file:///D:/dev2/kaedeniwa/crates/kaede-server/src/attachments.rs#L65-L80) | axum 先完成 body 提取才进 handler；应把认证/限流前移为中间件并限制并发 |
| P2-22 | 存储层健壮性 | [memory.rs:20-52](file:///D:/dev2/kaedeniwa/crates/kaede-storage/src/memory.rs#L20-L52)、[provider.rs:179](file:///D:/dev2/kaedeniwa/crates/kaede-mls/src/provider.rs#L179)、[idb.rs:171-177](file:///D:/dev2/kaedeniwa/crates/kaede-storage/src/idb.rs#L171-L177) | RwLock poison 直接 unwrap；IndexedDB clear 非原子；快照解析不校验尾部/重复 key |
| P2-23 | MLS 允许同一 uid 重复入组（待验证） | [group.rs:99-112](file:///D:/dev2/kaedeniwa/crates/kaede-mls/src/group.rs#L99-L112)、[group.rs:134-152](file:///D:/dev2/kaedeniwa/crates/kaede-mls/src/group.rs#L134-L152)、[collision.rs:41](file:///D:/dev2/kaedeniwa/crates/kaede-crypto/src/collision.rs#L41) | 同名 uid 被跳过碰撞检查，remove 只删第一个 leaf；需构造探针确认可达性 |
| P2-24 | 私密频道 group 无重建/重置路径 | [connection.rs:585-592](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/connection.rs#L585-L592)、[room.rs:987-1046](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/room.rs#L987-L1046) | 已有 group 时一律拒绝 Welcome；只有“单人 409”会重建。多人频道全员丢失快照后无法自愈（Web 端在 P0-1 下每次刷新都会命中） |

---

## 4. 已核对且未发现问题的区域

- **协议**：Client/Server 枚举方向隔离、版本上下界、顶层 16 MiB、`from` 由服务器注入。
- **密码学**：AEAD 长度前置校验、HKDF RFC 5869 KAT、SFrame 头部 AAD/计数器分区/`checked_next`、`SframeKey` 零化。
- **MLS**：`max_past_epochs=64`、credential UTF-8/uid 校验、exporter context 单射分隔、快照解析硬上限（除尾部校验）。
- **服务器**：SQL 全参数化、动态列名白名单、附件路径穿越防护（除 `-` 缺陷）、删除成员 rank 校验（op4 不能删 op5/op4）、默认频道幂等创建、媒体 seed TTL/空房回收。
- **客户端**：锁不跨 await、加锁顺序一致、WS 心跳 45s、重连/`ReportEpoch`、私密频道主群旁路在 UI 侧二次拦截。
- **UI**：身份字段用服务端 `from` 覆盖、频道可见性二次校验、XSS 面（文本节点转义、附件 mime 白名单）、时间显示已用本地时区。
- **媒体**：SFrame 接收端顺序/重放校验、远端数据长度校验、音频有界队列、`publish_screen` codec 映射。

---

## 5. 已知限制 / 未实现（本轮未计入缺陷）

- Web 语音实际上已接线（[voice_web.rs](file:///D:/dev2/kaedeniwa/crates/kaede-ui/src/voice_web.rs)），但 README 仍写“Web 语音/屏幕共享不可用”，文档已过时。
- 好友请求无实时推送；SfuToken/SDP/ICE 是协议遗留死路径；画质档位未完全接入媒体层。
- 私密频道反应不支持、离线授权依赖权威成员上线、DM 历史分页/搜索/已读未实现。
- `provider` 整包快照 O(n²) 体积、以及“发送成功但落盘前崩溃”的残余 nonce 复用风险仍需事务型 MLS storage 才能根治。

---

## 6. 建议修复顺序

1. **P0-1 / P0-2**：Web 身份持久化与 Web 语音 E2EE（安全边界，且影响所有 Web 用户）。
2. **P1-1 / P1-16**：私密频道回退泄露、DM/频道密文真实性。
3. **P1-5 / P1-6 / P1-7 / P1-8 / P1-9**：附件、CORS、账号吊销、离线信封、epoch 污染（部署与全服稳定性）。
4. **P1-2 / P1-3 / P1-4**：MLS 并发、KP 落盘、落盘失败阻断（群一致性与 nonce 复用）。
5. **P1-10 / P1-11**：跨账号隐私、移动端核心 Tab。
6. **P1-12 ~ P1-15**：媒体互通、track 生命周期、采集分辨率。
7. P2 按迭代排期，优先 P2-1（CI）、P2-8（丢消息）、P2-15（内存）。

---

## 7. 修复落地记录（2026-10-21）

### 安全边界

- **Web 快照**：`device_cipher` 在 wasm 且 DEK 可用时改用 WebCrypto DEK；读取端增加 DEK→fallback 迁移尝试（[keystore.rs](file:///D:/dev2/kaedeniwa/crates/kaede-client/src/keystore.rs)）。
- **Web 语音 E2EE**：改用 `livekit-client@2.14.0` 的 `e2ee: { keyProvider, worker }`；`index.html` 通过 fetch+Blob 创建同源 Worker；Web/native 统一 PBKDF2 hex 口令（[wasm_impl.rs](file:///D:/dev2/kaedeniwa/crates/kaede-livekit-web/src/wasm_impl.rs)、[index.html](file:///D:/dev2/kaedeniwa/crates/kaede-wasm/index.html)）。
- **媒体失败不降级**：exporter 失败直接拒绝连接；Web/native 均不再接受无 E2EE key 的媒体会话。
- **DM/频道密文**：升级为 v2（`0x05` 前缀），AAD 绑定 `conv_id + from + to + msg_id`；客户端维护 1024 条重放窗口，兼容旧 v1 历史（[dm_cipher.rs](file:///D:/dev2/kaedeniwa/crates/kaede-crypto/src/dm_cipher.rs)）。
- **私密频道**：缓存频道隐私属性；本地无 channel group 时禁止发送/上传附件，绝不回退主群；频道 group 支持按 group_id 重建。

### 群状态与持久化

- 所有 MLS epoch 变更（Add/Remove）串行化，Commit 使用生成时捕获的 epoch。
- KeyPackage 生成后立即标记 dirty；快照保存失败会 emit `MlsSyncError` 并阻断后续发送，直到存储恢复。
- 本地消息/历史明文缓存改为设备 cipher 加密，并在 key 前缀加入 uid，隔离跨账号数据。

### 服务端

- 附件 ID 白名单补上 `-`；删除 `main.rs` 重复 CORS 层。
- WS Join 重新校验 session token；连接每条消息 + 1s 空闲切片检查 peer 归属，踢人/顶替/删号立即断开。
- 离线信封：单条 256 KiB、每用户 4 MiB/100 条上限；校验 Welcome 目标与 ChannelSync 收件人；投递成功后再删除。
- **在线投递同样限额**：Welcome/ChannelSync 256 KiB、Commit/Proposal 2 MiB、AppMessage 4 MiB，避免在线成员直接互推 16 MiB 帧。
- `mls_commits` 限制 epoch 范围，保留策略改为按 rowid 而非 `MAX(epoch)`。
- 移除明文 token 兼容；私密频道改名校验成员；昵称双写事务 + 控制字符校验。
- LiveKit DM token 校验真实会话/账号并限流；各路由增加 body limit；认证头前置中间件。

### UI / 媒体 / 存储

- 移动端私聊/好友页补数据加载；语音频道在 Web 走 `voice_web`、在桌面 compact 布局走原生 `connect_voice`；去掉重复 Tab。
- 历史加载改为合并；加入弹窗状态复位；reaction 使用稳定 ts；滚动/回复/Toast/DM 时间/附件内联大小/平板成员按钮修复。
- `unpublish_track` 使用真实 publication sid；发布任务异常退出时清理 + 发事件；采集默认保持原生分辨率并按实际分辨率选编码器。
- Memory/MLS 锁 poison 恢复、IDB clear 单事务、快照尾部/重复 key 校验、同 uid 重复 Add 拒绝。

### 残余风险

- 重放窗口是内存态：进程重启后同一密文的极旧重放不再被拦截（AAD 的发送者/方向绑定仍然有效）。
- 旧 Web snapshot（fallback 写入）会在升级后首次读取时自动尝试迁移；若 DEK 不可用则仍会重建身份。
- Web E2EE worker 依赖 CDN 脚本可访问；离线 PWA 需把 worker 缓存到本地。
- 踢人/顶替的连接关闭延迟最多 1 秒。
- Web↔native 语音的 PBKDF2 参数已静态对齐，但仍需真机互通验证。
- 认证头前置只会拒绝“无 Authorization”请求；持有有效 token 的攻击者仍可在配额内缓冲大 body（已通过路由 body limit 大幅收敛）。

---

## 附录：验证命令

```powershell
cargo check --workspace --all-targets
cargo clippy -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server --all-targets -- -D warnings
cargo test -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server
cargo check -p kaede-wasm --target wasm32-unknown-unknown
cargo fmt --all -- --check
```

说明：P0/P1 结论均已在当前代码中逐行复核；P2-2、P2-22、P2-23 标注为需要运行时探针进一步确认，其余为静态可确认缺陷。
