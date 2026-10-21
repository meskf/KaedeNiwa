# KaedeNiwa 第二轮全量审计与修复规格（Spec）

- 审计日期：2026-10-21
- 规格状态：已实施（2026-10-21，范围 B 全部完成并通过回归）
- 审计对象：当前工作区（含未提交修改），不是 `HEAD`
- 代码规模：12 个 crate，约 2.9 万行 Rust
- 本轮性质：只读审计；本文档同时作为审计结论与修复规格

---

## 1. 基线（本轮实测）

| 项目 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo check --workspace --all-targets` | 通过，0 warning |
| `cargo clippy -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server --all-targets -- -D warnings` | 通过 |
| `cargo test -p <核心 6 crate>` | 214 passed / 0 failed / 2 ignored |
| wasm 目标检查 | 本轮未重跑（上一轮通过） |

结论：上一轮修复没有引入编译/测试回归；但仍有 6 个会直接导致功能失效或安全边界破坏的问题，以及一批 P2 和未实现功能。

---

## 2. 结论摘要

本轮新确认的主要问题：

1. **私密频道的聊天消息没有实时投递**。发送端只用频道密钥封装后写入 HTTP 历史，服务器不广播，其他成员必须重新拉历史才能看到。
2. **私密频道可被主群成员注入伪造消息**。主群 MLS 广播里只要 `payload.c` 等于某私密频道 id，且接收者能看到该频道，UI 就会把它插入该频道消息列表。
3. **私密频道的表情/媒体信令仍走主群广播**，向所有主群成员泄露频道 id、目标消息 ts、表情等内容。
4. **主群恢复路径永久分裂**：已有本地群的一方收到新的 Welcome 时直接忽略；房主丢失快照重建群后，guest 永远无法切到新群。
5. **DM 单侧重建群后永久分裂**：`dm_join_group` 的“group_id 最小者胜出”规则会让重建方的新 Welcome 被对端丢弃，且没有后续收敛机制。
6. **媒体房间 seed 生命周期实际失效**：客户端崩溃/关闭页面未调用 `/leave` 时，participants 永远非空，seed 永不轮换；服务器清理任务也不会回收这类房间。

另外，生产入口未创建默认频道，导致全新部署首次登录没有任何频道可用。

---

## 3. 已验证缺陷

### 3.1 P0 / P1

#### R2-01 私密频道消息无实时投递（P1，功能失效）

- 位置：
  - `crates/kaede-client/src/room.rs:1218-1236`、`room.rs:1276-1304`（私密频道分支 `0x03` 封装后仅 `post_message`）
  - `crates/kaede-server/src/http.rs:448-521`（`post_message` 只 INSERT，不广播）
  - `crates/kaede-ui/src/content.rs:108-161`（仅切换频道时拉一次历史）
- 机制：私密频道不走 `mls_send_app`，而 `post_message` 没有任何 WS fan-out。
- 影响：A 在私密频道发言，B 在当前频道界面里看不到，直到刷新/重连/切换频道。实时聊天等于不可用。
- 证据：全仓 `ChannelSync` 仅用于 Welcome/Commit；`post_message` 内无 `broadcast`/`send_to`。

#### R2-02 私密频道消息注入（P1，完整性）

- 位置：
  - `crates/kaede-client/src/connection.rs:1405-1446`（主群 `MlsAppMessage` 解密后直接发 `AppMessage`）
  - `crates/kaede-ui/src/observer.rs:125-146`（只检查 `payload.c` 是否在可见频道列表）
- 机制：任意主群成员可构造 `{"c":"ch_xxx",...}`，接收者只要是该私密频道成员且频道在列表中，就会写入该频道消息列表。
- 影响：伪造私密频道消息、污染 UI；不进入服务器历史，但用户无法区分。
- 备注：身份字段 `u` 已被 `from` 覆盖，但消息体/频道归属未校验。

#### R2-03 私密频道表情与媒体信令经主群广播（P1，机密性）

- 位置：
  - `crates/kaede-client/src/room.rs:1309-1332`（`send_reaction` 恒走 `mls_send_app`）
  - `crates/kaede-client/src/room.rs:1674-1712`（`publish_media_meta` / `publish_media_stop` 恒走主群）
- 影响：任何主群成员可解密并读到私密频道的频道 id、目标 ts、表情、媒体元数据；表情与私密频道语义不一致。
- 备注：私密频道/DM 媒体当前已关闭 E2EE（C14），媒体元数据应直接禁用而非主群广播。

#### R2-04 已就绪方忽略新 Welcome，主群无法恢复（P1）

- 位置：`crates/kaede-client/src/connection.rs:1298-1311`（`Some(_) => warn + 忽略`）
- 触发：房主本地快照丢失后 `become_host()` 重新建群，再 `host_add_peer`；guest 本地仍有旧群。
- 影响：guest 忽略新 Welcome，随后收到新群的 Commit 无法处理；双方永久不同群，只能清空本地存储。注释已承认“需要手动重连”，但重连仍会恢复同一旧快照，实际无法自愈。

#### R2-05 DM 单侧重建群后永久分裂（P1）

- 位置：
  - `crates/kaede-client/src/connection.rs:657-686`（`dm_join_group` 以 group_id 字典序最小者胜出）
  - `crates/kaede-client/src/room.rs:1453-1478`（`dm_ensure_group` 重建后只发一次 Welcome）
- 触发：A 本地 DM 快照丢失 → 新建 group_A 并发 Welcome；B 已有 group_B，比较后保留较小者。
- 影响：若 B 保留旧 group_B，A 已在自己新 group 上发送，双方密钥永久不一致；没有重传/重新协商路径。并发首次建群也会丢先发的首条消息。

#### R2-06 媒体房间 seed 不轮换 / 内存泄漏（P1，密码学 + 资源）

- 位置：
  - `crates/kaede-server/src/state.rs:76-80`、`state.rs:132-139`（`participants` 非空即永久保留）
  - `crates/kaede-server/src/livekit.rs:76-94`（每次发 token 插入 participant）
  - `crates/kaede-server/src/livekit.rs:162-184`（只有显式 `/leave` 才移除）
- 触发：关闭页面、进程崩溃、网络中断，客户端来不及调用 `/leave`。
- 影响：
  1. seed 永久复用；LiveKit SDK ratchet 重新从 0 开始时可能产生同 key/nonce（P0-Z 试图规避的问题重新出现）。
  2. `room_media_seeds` 无界增长。
- 说明：这不是“下一次进入一定复用”，而是“只要有一个残留 participant，seed 就一直复用”。

#### R2-07 生产入口未创建默认频道（P1，首次部署不可用）

- 位置：
  - `crates/kaede-server/src/lib.rs:88`（只在 `spawn_test_server` 调用）
  - `crates/kaede-server/src/lib.rs:110-143`（`ensure_default_channels` 为私有函数）
  - `crates/kaede-server/src/main.rs:46-49`（启动路径未调用）
  - `crates/kaede-client/src/room.rs:45`（默认频道 `general`）
- 证据：migrations 中没有任何 `INSERT INTO channels`。
- 影响：全新数据库启动后 `list_channels` 为空；发送到 `general` 返回 404；UI 无频道可进。

### 3.2 P2（健壮性 / 体验 / 数据卫生）

| 编号 | 问题 | 位置 | 影响 |
|---|---|---|---|
| R2-08 | 删除成员只清 members/accounts/sessions/KP/friends | `server/src/members.rs:297-323` | `channel_members` 残留（私密频道权威选择会选中已删用户）、`dm_conversations` 与 `pending_envelopes` 残留 |
| R2-09 | 私密频道无在线权威成员时加入必然超时 | `server/src/channels.rs:536-568`、`client/src/room.rs:1010-1031` | 创建者离线时新成员永远无法入群，且 10s 后 UI 报错；服务端没有补投 `ChannelMemberJoined` |
| R2-10 | 私密频道自己发的消息重载后重复 | `client/src/room.rs:1240`、`room.rs:1297`，配合 `decrypt_history_record` 可解密自己的 0x03 消息 | 本地副本用客户端 ts、服务器记录用服务器 ts，去重条件不命中 |
| R2-11 | 本地消息存储 key 仅秒/毫秒 ts | `client/src/room.rs:634-637` | 同毫秒两条消息后者覆盖前者 |
| R2-12 | 表情发送失败被静默吞掉 | `ui/src/content.rs:411-413`、`content.rs:709-711` | 用户以为已添加，实际未广播 |
| R2-13 | 附件以 data URL 全量 base64 渲染 | `ui/src/attachment.rs:63-90` | 32MiB 附件 => >42MiB 字符串，多附件时页面内存暴涨 |
| R2-14 | DM 历史无分页/无“加载更早” | `ui/src/dm_view.rs:8-37` | 只能看最近 100 条 |
| R2-15 | UI 无“退出私密频道”入口 | `client/src/room.rs:1038-1042` 有 API；`kaede-ui` 无调用 | 成员只能被删频道或删本地数据 |
| R2-16 | 语音成员列表是假数据 | `ui/src/sidebar.rs:791-799` | 永远只有“我”，不反映真实 participant |
| R2-17 | wasm 下语音按钮只切换 UI 状态 | `ui/src/sidebar.rs:1039-1043`、`:1112-1116` | Web 端语音/屏幕共享实际不可用 |
| R2-18 | 画质档位未接入媒体层 | `ui/src/state.rs:300-301` 注释、`sidebar.rs:1056` | 选择档位只影响日志 |
| R2-19 | `SfuTokenRequest` 未实现，SDP/ICE 无客户端调用 | `server/src/ws.rs:746-751` | 协议面存在死路径（LiveKit 路线替代） |
| R2-20 | 好友请求无实时推送 | `server/src/friends.rs` 无 WS 事件；UI 仅刷新列表 | 对方请求需手动刷新才能看到 |
| R2-21 | 附件 `thumbnail_url` 永远为 `None` | `server/src/attachments.rs:113-120` | 无缩略图 |
| R2-22 | 登录失败仅对存在的账号跑 Argon2 | `server/src/http.rs:177-196` | 可通过响应时间枚举账号 |
| R2-23 | 每个 DM/频道快照包含整个 provider storage | `mls/src/provider.rs:187-234`、`client/src/connection.rs:769-872` | 快照体积随 group 数近似平方增长，可能触及 16MiB 上限 |
| R2-24 | 历史加载标记在 spawn 前写入 | `ui/src/content.rs:126-130` | 加载失败后本次会话不再重试（重连会清理） |

---

## 4. 未实现 / 半成品功能清单

以下为代码中确认的缺口，不属于本轮修复 bug，但需在产品排期中明确：

1. 私密频道实时消息（R2-01）。
2. 私密频道表情/反应（当前错误地复用主群，R2-03）。
3. 私密频道主动退出 UI（R2-15）。
4. 私密频道离线授权加入（R2-09）。
5. DM 历史分页与本地历史缓存（R2-14）。
6. 附件上传进度、断点/重试、缩略图（R2-13、R2-21）。
7. 好友请求实时通知（R2-20）。
8. 语音成员列表与 participant 状态同步（R2-16）。
9. Web 端语音/屏幕共享（R2-17）。
10. 8K/4K/码率档位接入媒体层（R2-18）。
11. SfuTokenRequest / SDP / ICE 信令（R2-19，建议删除或标注废弃）。
12. 频道成员管理 UI（查看成员列表/成员权限）未落地。
13. 消息搜索、已读、未读、@ 提醒未实现。
14. 服务端会话/设备管理（多设备登录列表、远程登出）未实现。
15. 移动端入口（README 目标）无对应 crate/实现。

---

## 5. 修复方案（决策完成）

### F1（对应 R2-01）私密频道消息实时分发

**协议**
- `crates/kaede-protocol/src/lib.rs`：新增
  `ServerMessage::ChannelMessage { channel_id: String, from: UserId, ts: i64, ciphertext: Vec<u8> }`。
- 同步提升 `PROTOCOL_VERSION` 与 `MIN_SUPPORTED_VERSION`（旧客户端遇到未知 CBOR 变体会断开，必须显式版本隔离）。

**服务端**
- `crates/kaede-server/src/http.rs::post_message`：
  - 私密频道分支在 INSERT 成功后查询 `channel_members`，向除发送者外的每个成员 `state.send_to(ChannelMessage{...})`。
  - 复用 `ts` 返回值；发送失败（离线/队列满）忽略，离线成员靠历史补齐。
- 不改变公开频道行为（公开频道已由客户端 `mls_send_app` 实时广播）。

**客户端**
- `crates/kaede-client/src/connection.rs::handle_server_message` 新增分支：
  - 校验 `self.channel_has_group(&channel_id)`；
  - 用 `channel_export_key` + `kaede_crypto::dm_cipher::open` 解密 `ciphertext[1..]`（要求首字节 `0x03`）；
  - emit 新事件 `ClientEvent::ChannelMessage { server_id, channel_id, from, ts, plaintext }`。
- `crates/kaede-client/src/event.rs` 增加该事件。
- `crates/kaede-ui/src/observer.rs::apply_event` 处理该事件：解码 `MessagePayload`，强制 `channel_id = 事件 channel_id`，以服务器 `ts` 写入 `app.messages`，按 `(ts, author)` 去重。

**验收**
- 两个客户端加入同一私密频道，A 发送后 B 在 1s 内出现消息，且刷新后不重复。
- 离线成员上线拉历史仍能看到该消息。

### F2（对应 R2-02 / R2-03）封闭私密频道的主群旁路

- `crates/kaede-client/src/connection.rs` 的 `MlsAppMessage` 分支：
  - 解码为 `Message` 或 `Reaction` 后，若 `p.c` 满足 `channel_has_group(&p.c)`，直接丢弃并 `warn`（私密频道内容只能来自 F1 的 `ChannelMessage` 或历史）。
- `crates/kaede-ui/src/observer.rs::apply_event` 的 `AppMessage` 分支：
  - 仅在频道 `!is_private` 时接受消息/反应，作为第二道防线。
- 私密频道表情：UI 在 `content.rs` 的 hover/emoji picker 中隐藏反应入口；`room.rs::send_reaction` 对 `channel_has_group(channel_id)` 返回 `ClientError::Protocol("private channel reactions are not supported yet")`，并提示用户。
- 私密频道媒体元数据：`publish_media_meta/publish_media_stop` 在 `channel_has_group` 或 `dm_has_group` 时直接返回 `Ok(())` 并 `warn`（LiveKit 路径不依赖该信令）。

**验收**
- 主群成员构造 `c=<私密频道>` 的 AppMessage，私密频道成员 UI 不出现消息/反应。
- 私密频道内点表情得到明确提示，主群成员收不到任何表情广播。

### F3（对应 R2-04）Welcome 驱动的主群重建

- `crates/kaede-client/src/mls_session.rs`：新增 `reset_and_join(&mut self, welcome: &[u8]) -> Result<(), ClientError>`：
  - 清空 `group`/`group_id`，复用同一 `identity`/`provider` 执行 `join_with_welcome`。
- `crates/kaede-client/src/connection.rs`：新增 `mls_join_or_reset_welcome(&self, welcome) -> Result<bool, ClientError>`：
  - 未就绪：正常 join；
  - 已就绪：用 `fork_for_new_group()` 先解析 Welcome 得到新 `group_id`；相同则返回 `false`（重复投递）；不同则 `reset_and_join` 替换主群，`mark_main_dirty()`，返回 `true`。
- `handle_server_message(MlsWelcome)` 使用该函数；发生重建时 emit `MlsSyncError { recoverable: true, user_message: "服务器主群已重建，本地旧群历史不可恢复" }` 并保持 `MlsReady`。

**验收**
- 单元测试：host 重建群并 Add 已 Ready 的 guest，guest 的 `group_id` 变为新群、epoch 对齐、双向消息可解。
- 重复 Welcome 不触发重建。

### F4（对应 R2-05）DM 代际仲裁

- Welcome 信封改为 `0x02 || generation:u64 BE || welcome`（`client/src/room.rs::dm_ensure_group`、`connection.rs::handle_server_message(DmReceived)`、`room.rs::dm_load_history` 三处同步）。
- 新增持久化命名空间 `dm_meta`（`client/src/connection.rs`，key=`{server_url}/{conv_id}`）：存 `(generation, group_id)`。
- `dm_init_group`：`generation = local + 1`，先持久化 meta，再发送 Welcome。
- `dm_join_group(conv_id, welcome, generation)`：
  1. `generation > local` → 采纳新群；
  2. `generation < local` → 忽略；
  3. 相等 → 维持现有 group_id 字典序最小仲裁（仅并发首次建群）。
- 采纳/丢弃时 emit 可解释事件；历史密钥不可用时提示“该会话旧消息不可恢复”。

**验收**
- 单元测试：A 正常建群 gen=1；B 丢失本地状态后重建 gen=2 并发送 Welcome；A 采纳 gen=2，双方双向消息可解。
- 并发首次建群仍收敛到同一 group。

### F5（对应 R2-06）媒体 seed 生命周期

- `server/src/state.rs`：`RoomMedia.participants: HashMap<String, Instant>`。
- `server/src/livekit.rs::issue_token`：`participants.insert(uid, Instant::now())`。
- `leave_room`：`remove`。
- 清理任务：
  - 先删除 `last_seen > MEDIA_PARTICIPANT_TTL`（建议 8h，大于 token 6h 有效期）的 participant；
  - participants 为空且 `last_used > 1h` 时删除 room seed。
- 保持“同一通话内所有人拿同一 seed”的语义。

**验收**
- 单元测试：模拟最后一个 participant 超过 TTL 未续期，seed 被回收；下次进入生成新 seed。
- 正常 `/leave` 仍立即回收。

### F6（对应 R2-07）启动时创建默认频道

- `server/src/lib.rs`：`ensure_default_channels` 改为 `pub(crate)`。
- `server/src/main.rs`：`db::init` 之后调用 `ensure_default_channels(&pool)`，打印创建的频道数。
- 保持幂等（当前 `COUNT(*)>0` 直接返回）。

**验收**
- 空数据库启动后 `GET /api/channels` 至少返回 `general`（text）与 `voice`（voice）。

### F7（对应 R2-08）删除成员的数据清理

- `server/src/members.rs::remove_member` 事务内追加：
  - `DELETE FROM channel_members WHERE login_id = ?`
  - `DELETE FROM pending_envelopes WHERE to_uid = ? OR from_uid = ?`
  - `DELETE FROM dm_conversations WHERE a = ? OR b = ?`
- 消息与附件保留（审计/历史需要），在文档中明确。

**验收**
- 集成测试：删除成员后三张表无残留，频道权威选择不再选中该用户。

### F8（对应 R2-09）离线权威补投

- `server/src/channels.rs::notify_authority`：无在线成员时，向第一个成员
  `enqueue_envelope(kind="channel_member_joined", payload = JSON{requester, key_package})`。
- `server/src/ws.rs::deliver_pending`：新增该 kind → `ServerMessage::ChannelMemberJoined`。
- `client/src/room.rs::join_channel` 超时文案改为“当前无在线成员可授权，待其上线后自动完成”，并保留 pending 等待槽（不再清除），收到迟到 Welcome 后 emit `ClientEvent::ChannelJoined` 供 UI 提示并刷新。

**验收**
- 创建者离线时成员加入不丢请求；创建者上线后收到事件并补发 Welcome，新人自动入群。

### F9（对应 R2-10 / R2-11 / R2-12）

- `send_text_with_reply` / `send_text_with_attachments`：仅在 `!channel_has_group(channel_id)` 时 `persist_local_message`。
- `local_msg_key` 增加 4 位随机后缀（`{ts:013}-{nonce}`），避免同毫秒覆盖。
- `content.rs` 两处表情发送失败改为 `ui.toast` 提示。

### F10（对应 R2-13 / R2-14 等 P2，可分批）

- 附件：>8MiB 不使用 data URL，改为下载 Blob/文件链接；小于阈值保留内联预览。
- DM：增加 `dm_load_history` 的 `before_ts` 分页与“加载更早”按钮。
- UI：私密频道右键菜单增加“退出频道”，调用 `leave_channel` 并从列表移除。
- 语音成员：订阅 `MediaEvent::ParticipantConnected/Disconnected` 更新 `VoiceChannel.members`。
- 文档标注：wasm 语音、SfuToken/SDP、画质档位、好友实时通知为当前限制。

---

## 6. 验证计划

实现完成后必须全部通过：

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server --all-targets -- -D warnings
cargo test -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server
cargo test -p kaede-client --features native
cargo check -p kaede-wasm --target wasm32-unknown-unknown
cargo check -p kaede-client --features wasm --target wasm32-unknown-unknown
```

新增回归测试至少覆盖：

1. 私密频道实时消息（服务端集成 + UI 事件路径）。
2. 私密频道主群旁路注入被拒绝。
3. 主群 Welcome 重建（旧群 → 新群）。
4. DM gen=2 重建收敛。
5. 媒体 seed TTL 回收。
6. 空库默认频道。
7. 删除成员孤儿清理。
8. 离线权威补投。

---

## 7. 已知残余风险（不在本轮修复范围）

- 快照写入失败或进程在“发送成功后、落盘前”崩溃，仍可能导致 ratchet 回退；接收端会因 `SecretReuseError` 拒绝消息。彻底解决需事务型 MLS storage。
- 私密频道旧历史中已经用主群密钥加密的 0x03 之前的消息无法重新加密，只能继续隐藏。
- 自动重连依赖内存中的 session token 或登录时保存的密码；两者都不可用时仍需手动登录。
- `provider` 整包快照的 O(n²) 体积问题（R2-23）需要后续存储改造。
- 私密频道/DM 语音 E2EE 已于 **v0.0.1-alpha.2 开启**（C14 落地）。
  三条路径（DM / 私密频道 / 主群）各自从本地 group state 派生 key，
  room-level seed 统一注入，同一媒体会话内所有参与者 key 一致。
  残余限制（块 B 已收口，v0.0.1-alpha.2 后续 commit）：
  - 主群：`ClientEvent::MlsEpochChanged` → observer 自动重连 ✅
  - 私密频道：`ClientEvent::ChannelEpochChanged` → observer 自动重连 ✅
  - DM：无 epoch 变化路径（2 人固定 group，无 add/remove/self_update），
    只有"一方删群重建"会改变 key —— 属换 group 语义，用户手动重进。

---

## 8. 需要用户批准

请选择本轮实现范围：

- **A（推荐）**：R2-01 ~ R2-09 + F9（安全/功能阻断项）。
- **B**：A + P2 中 R2-13 ~ R2-17（体验项）。
- **C**：仅 R2-01 ~ R2-07。

批准后按 `tasks.md` 顺序实现并回归。
