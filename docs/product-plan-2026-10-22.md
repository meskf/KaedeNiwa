# KaedeNiwa 产品级未实现功能 — 实施方案（待批准）

- 日期：2026-10-22
- 状态：方案评审中，**未开始实现**
- 目标：把上一轮列出的 8 项产品级缺口落地，同时保持 E2EE / 协议 / 数据兼容性可控
- 当前基线：协议 v4；核心测试 228 passed；Web/Desktop 可用，无独立移动端 crate

---

## 0. 先明确约束（会直接影响方案）

1. **E2EE 约束**：服务端只存密文，**不能做全文搜索、不能生成附件缩略图、不能解析 @**。
   因此搜索、@、缩略图必须是客户端能力；服务端只能存不透明密文或做元数据统计。
2. **协议版本**：新增/删除 `ClientMessage`/`ServerMessage` 变体会破坏旧客户端 CBOR 解码。
   本轮所有协议变更合并为 **v5**，`MIN_SUPPORTED_VERSION = 5`；服务端与客户端必须同版本部署。
3. **单设备在线**：当前同账号新 WS 会顶替旧连接（`add_peer` 踢旧）。真正的“多设备同时在线”
   需要改 presence/peer/MLS 广播模型，属于架构级改动；本轮默认**不做**。
4. **附件**：客户端加密后上传，服务端只存密文；大小上限 32 MiB，无缩略图、无进度、无断点。
5. **移动端**：现有 `mobile_pages`/`mobile_shell` 是 Web 响应式布局 + PWA，不是 Android/iOS 原生 crate。
6. **数据库**：SQLite migrations 按时间戳顺序追加；客户端本地存储为 SQLite/IndexedDB，需同步迁移。

---

## 1. 范围与分批

| 批次 | 功能 | 复杂度 | 依赖 |
|---|---|---|---|
| 1 体验快赢 | 好友请求实时推送；8K/4K 档位真正生效；私密频道成员列表/退出 UI | S/M/M | 协议 v5（好友事件） |
| 2 消息体验 | 已读/未读；@ 提醒；本地消息搜索 | M/L/M | 新表 + 客户端本地索引 |
| 3 管理与附件 | 服务端会话/设备管理；附件上传进度；附件缩略图；频道成员移除（含 MLS 协调） | M/L/L/L | 协议 v5（会话/频道事件） |
| 4 清理与移动端 | SfuToken/SDP/ICE 清理；移动端方案（默认 PWA，可选 Dioxus Mobile spike） | M/XL | 协议 v5 |

建议按批次交付，每批独立可回归、可回滚；**协议 v5 在第 1 批一次性升级**，后续批次只加新变体不再改版本。

---

## 2. 批次 1：体验快赢

> **状态：已完成（2026-10-22）**。规格与任务见 `docs/product-batch1/`。
> 协议已升到 v5；新增测试 `friend_changes_push_realtime`、`channel_members_visibility`、
> `friends_changed_round_trip`；全量回归通过。

### 2.1 好友请求实时推送

**现状**：`friends.rs` 只有 HTTP；`ClientEvent` 无好友事件；`friends_view.rs` 靠手动刷新。

**方案**
- 协议新增 `ServerMessage::FriendsChanged`（无业务负载，仅触发刷新）。
  用无负载事件而不是直接推 `Friend` 结构，避免把 server HTTP DTO 耦合进协议。
- 服务端：`send_request` / `accept_request` / `remove_friend` / 删除成员后，对相关 uid
  `state.send_to(FriendsChanged)`；离线不补投（DB 已持久化，登录即拉取）。
- 客户端：`connection.rs` 新增分支 → `ClientEvent::FriendsChanged`；`observer` 把它转成
  UI 信号，`friends_view` 自动 `load_friends_into`。
- 验收：
  - A 给在线 B 发好友请求，B 的“待处理”列表 1s 内出现，无需刷新；
  - B 离线时请求不丢，下次登录列表正确；
  - 接受/删除同样实时刷新。

**复杂度**：S；**风险**：低。

### 2.2 8K/4K 画质档位真正生效

**现状**：native 侧栏已经把 `profile.width/height/fps/bitrate` 传入 `VideoConfig`；
但 `state.rs` 注释仍写“未接入 media”，Web `getDisplayMedia` 没传分辨率；档位切换不会影响进行中的共享。

**方案**
- Native：
  - 核对 `VideoProfile` 档位表，确保 4K/8K 档位存在且 bitrate 合理；
  - 档位切换时若正在屏幕共享，提示“下次共享生效”或自动 unpublish→publish（推荐自动重建）；
  - 8K 档位在硬件不支持时给出明确 Error 事件，不静默黑屏。
- Web：
  - `voice_web::set_screen_share` 的 `getDisplayMedia` 增加 `width/height/frameRate` 约束
    （`width: { ideal: profile.width, max: profile.width }`）；
  - 浏览器不支持目标分辨率时回退到实际分辨率并 toast；
  - 码率无法直接约束时，使用 `RTCRtpSender.setParameters` 调整 `maxBitrate`（可选）。
- 清理 `state.rs` 的过时注释和 README“画质档位未接入”条目。
- 验收：
  - 选择 3840×2160@60 → native capture 输出 4K，SDP 通告 4K；
  - Web 选择 4K → `getDisplayMedia` 约束为 4K；
  - 选择 8K 时按 AV1/HEVC 选编码器，失败有可读错误。

**复杂度**：M；**风险**：中（浏览器/硬件差异）。

### 2.3 私密频道成员列表 / 退出 UI

**现状**：`GET/POST/DELETE /api/channels/{id}/members` 目前只有 join/leave 能力；
UI 没有“查看成员”“退出频道”入口。

**方案**
- 服务端新增 `GET /api/channels/{id}/members`：
  - 公开频道：所有登录用户可看；
  - 私密频道：仅 `channel_members` 成员可看；
  - 返回 `login_id/nickname/joined_at`。
- UI：频道右键菜单/详情面板增加“成员”与“退出频道”；退出调用现有 `leave_channel` 并从列表移除。
- 移除他人（owner/op5）放到批次 3，因为它需要 MLS `remove_member` + 权威协调。
- 验收：成员列表可见；退出后频道从本人列表消失；服务端 `channel_members` 同步删除。

**复杂度**：M；**风险**：低。

---

## 3. 批次 2：消息体验

> **状态：已完成（2026-10-22）**。
> - 新增迁移 `20261024000001_reads.sql` 与 `/api/reads`、`/api/unread`；
> - `MessagePayload.m` 携带 @ 提及，客户端解析/高亮并累加 mention 未读；
> - `RoomSession::search_local` + 侧栏“搜索本地消息”弹窗；
> - 新增测试 `unread_counts_track_reads`；全量回归通过。

### 3.1 已读 / 未读

**现状**：无 `last_read`、无未读计数、无红点。

**方案**
- 迁移：
  ```sql
  CREATE TABLE channel_reads (
    channel_id TEXT NOT NULL, login_id TEXT NOT NULL,
    last_read_ts INTEGER NOT NULL,
    PRIMARY KEY (channel_id, login_id)
  );
  CREATE TABLE dm_reads (
    conv_id TEXT NOT NULL, login_id TEXT NOT NULL,
    last_read_ts INTEGER NOT NULL,
    PRIMARY KEY (conv_id, login_id)
  );
  ```
- API：
  - `POST /api/reads` `{ channel_id?, conv_id?, ts }`（批量可选）；
  - `GET /api/unread` → 每个频道/会话的未读条数（SQL `COUNT(*) WHERE ts > last_read AND sender != me`）。
- 客户端：
  - 打开频道/会话、窗口重新聚焦时上报 `last_read_ts = 当前最大 ts`；
  - `AppState` 增加 `unread: HashMap<key, u32>`；
  - 侧栏/会话列表显示红点或数字。
- 实时：新消息到达时本地未读数 +1；不做“逐条已读回执”（避免隐私和复杂协议），
  只做**未读计数**，后续可扩展 `ReadReceipt`。
- 验收：刷新后未读数一致；打开频道清零；自己发的消息不计未读。

**复杂度**：M；**风险**：低。

### 3.2 @ 提醒

**现状**：payload 无 mention 字段；UI 不解析 @。

**方案**
- `MessagePayload` 增加 `m: Vec<String>`（mentions，E2EE 内部字段）。
- 发送时客户端解析 `@login_id`（只匹配频道成员/好友），写入 mentions。
- 接收端：
  - `observer` 渲染时高亮 mention；
  - 若 mentions 包含自己且当前不在该频道，客户端本地未读 mention 计数 +1（从解密后的消息计算）；
  - 可选：浏览器/桌面通知，需用户授权。
- 服务端**不解析** mentions（E2EE），只负责存密文/未读总数。
- 验收：@ 高亮；被 @ 时侧栏出现 mention 标记；刷新后仍可识别（依赖本地已解密缓存）。

**复杂度**：M；**风险**：中（需要客户端能拿到最近消息用于 mention 统计）。

### 3.3 本地消息搜索

**现状**：无搜索 API；服务端只有密文，无法全文搜索；客户端有 `history_cache`。

**方案（推荐：纯本地搜索）**
- 搜索范围：已加载消息 + `history_cache` 中已解密的记录；不承诺搜索未缓存/未解密的历史。
- 实现：
  - `RoomSession` 暴露 `search_local(query, scope) -> Vec<SearchHit>`；
  - 对每个频道/DM 的缓存做大小写不敏感子串匹配，按 ts 排序；
  - UI 搜索面板：输入框 + 结果列表（作者、时间、片段、频道），点击跳转并加载上下文；
  - 结果上限（如 200 条）防卡顿。
- 可选增强：搜索时按需拉取并解密更多历史（受 MLS 重放限制，需走频道静态 key / 缓存）。
- 明确提示：“仅搜索本地已解密历史”，避免误导。
- 验收：中文/英文子串命中；点击结果能定位消息；无缓存时给出提示。

**复杂度**：M；**风险**：中（历史覆盖范围）。

---

## 4. 批次 3：管理与附件

> **状态：已完成（2026-10-22）**。
> - `sessions` 增加设备名/时间/公开 session_id；新增 `/api/sessions` 列表、撤销单个/其他；
>   设置面板增加“登录设备”；
> - 附件上传：native `reqwest::Body::wrap_stream` 分块进度，wasm `XHR upload.onprogress`，
>   全局进度条；
> - 图片缩略图：上传时客户端生成（native `image` / wasm canvas），同 key 加密后作为第二个附件上传；
> - 私密频道成员移除：op5 接口 + `ChannelRemoved` 事件 + 权威成员 MLS remove + Commit；
> - 新增测试 `sessions_list_and_revoke`、`remove_channel_member_notifies`；全量回归通过。

### 4.1 服务端会话 / 设备管理

**现状**：`sessions` 只存 token hash + login_id + expires_at；无列表/撤销 API/UI；
同账号新 WS 顶替旧连接，实际是**单设备在线**。

**方案（推荐：会话管理 + 保持单设备在线）**
- 迁移：`sessions` 增加 `device_name TEXT`、`created_at INTEGER`、`last_seen_at INTEGER`。
- 登录/WS Hello 上报设备名（native：hostname/OS；wasm：浏览器/OS）。
- API：
  - `GET /api/sessions` 列出当前账号会话（不返回 token，只返回 id/设备/时间/是否当前）；
  - `DELETE /api/sessions/{id}` 撤销；
  - `POST /api/sessions/revoke-others` 撤销其他；
  - 撤销当前会话时同时 `take_peer` 断开 WS。
- UI：设置面板“登录设备”列表 + 撤销按钮。
- 明确限制：不实现多设备同时在线；若要，另立架构项目（peer 表/ presence/ MLS 广播模型都要改）。
- 验收：列表正确；撤销其他设备后其 HTTP 401、WS 断开；当前设备不受影响。

**复杂度**：M；**风险**：低-中。

### 4.2 附件上传进度

**现状**：`upload_attachment` 一次性 `reqwest` 上传，无进度事件；Web 用 fetch 也没有上传进度。

**方案**
- Native：`reqwest` 使用 `Body::wrap_stream`，在读取流上累计已发送字节，通过回调/事件
  `ClientEvent::AttachmentProgress { name, sent, total }` 推给 UI。
- Wasm：改用 `XmlHttpRequest` 的 `upload.onprogress`（或 gloo-net 的 XHR），封装一个
  `upload_with_progress`；保持与 native 相同的进度事件。
- UI：输入框附件 chip 显示进度条；失败显示重试按钮。
- 断点续传：本轮不做（需要服务端分片上传 + offset 表）；只做整文件重试。
- 验收：大文件上传时进度平滑；失败可重试；完成后消息正常发送。

**复杂度**：L；**风险**：中（wasm XHR 与现有 reqwest 抽象并存）。

### 4.3 附件缩略图

**现状**：`thumbnail_url` 永远 `None`；服务端只存密文，无法生成缩略图。

**方案（推荐：客户端生成加密缩略图）**
- 上传图片时客户端生成缩略图（native `image` crate；wasm `createImageBitmap` + canvas），
  用同一附件 key（File purpose）加密后作为独立附件上传。
- 消息里的 `Attachment.thumbnail_url` 指向缩略图附件 id；UI 优先下载/解密缩略图展示，
  点击再下载原图。
- 服务端零改动（仍然 opaque），仅需允许第二个附件上传。
- 非图片/视频不生成缩略图；大图才生成（如 >512KB），小图继续内联。
- 验收：32 MiB 图片不再生成 43 MiB data URL；列表只加载缩略图；点击可看原图。

**复杂度**：L；**风险**：中（wasm 图像处理兼容性）。

### 4.4 频道成员移除（含 MLS 协调）

**现状**：私密频道有频道独立 MLS group；成员变更必须由权威成员执行，否则群分裂。

**方案**
- 服务端：owner/op5 `DELETE /api/channels/{id}/members/{login_id}`：
  - 删除 `channel_members`；
  - 通知频道权威成员（最早加入者）执行 MLS `remove_member` 并广播 Commit；
  - 与现有 `ChannelMemberLeft` / `notify_authority` 复用。
- 客户端：权威成员收到 `ChannelMemberRemoved` 后 `channel_remove_member` + `ChannelSync` 广播；
  被移除者本地删除 channel group。
- UI：成员列表中的“移除”按钮（仅 owner/op5）。
- 验收：被移除者无法再解密新消息；其余成员 epoch 一致；离线权威上线后补执行。

**复杂度**：L；**风险**：高（MLS 权威/epoch 协调）。

---

## 5. 批次 4：清理与移动端

> **状态：已完成（2026-10-22）**。
> - 删除协议中的 SDP/ICE/SfuToken 变体、服务端分支、客户端事件与相关测试
>   （定向消息测试改用 `MlsWelcome`）；
> - PWA：Service Worker 缓存壳/静态资源/LiveKit SDK 与 E2EE worker，导航离线回退；
>   manifest 增加 `id`/`display_override`；index.html 补移动端 meta 与安全区；
> - 移动端顶栏增加本地搜索入口；不支持 `getDisplayMedia` / `getUserMedia` 的浏览器给出明确提示；
> - 真机验收清单：`docs/pwa-mobile-checklist.md`。

### 5.1 SfuToken / SDP / ICE 清理

**现状**：协议仍有 `SdpOffer/SdpAnswer/IceCandidate/SfuTokenRequest`；生产代码无调用，
只剩 server 转发、client 事件和测试；LiveKit HTTP token 已替代。

**方案（推荐：直接删除 + 协议 v5）**
- 删除协议变体、server 分支、client 事件/分支、相关测试；
- 依赖这些变体做“定向消息”测试的用例改用 `MlsWelcome` 或其他定向变体；
- `PROTOCOL_VERSION = 5`、`MIN_SUPPORTED_VERSION = 5`；
- README 标注旧客户端必须升级。
- 备选：只标记 deprecated 保留变体（风险低但继续留死路径）。
- 验收：协议 round-trip 测试更新；全量编译/测试通过；无生产调用残留。

**复杂度**：M；**风险**：中（破坏性协议升级）。

### 5.2 移动端

**现状**：只有 `kaede-wasm`（PWA/响应式）和 `kaede-desktop`；没有 Android/iOS crate。

**方案（推荐：PWA 优先，Dioxus Mobile 单独 spike）**
- 1）PWA 补完（推荐先做）：
  - 校验 manifest/service worker/离线缓存/安全区/移动端手势；
  - Web 语音/屏幕共享在移动浏览器上的可用性提示；
  - 真机验收清单（iOS Safari / Android Chrome）。
- 2）Dioxus Mobile（可选，单独立项）：
  - 新建 `kaede-mobile` crate，评估 Android/iOS 工具链；
  - 麦克风/屏幕共享权限、LiveKit native（Rust SDK 的移动端支持）、后台音频；
  - 先做 1 周 spike（可运行空壳 + 登录 + 文本聊天），再决定是否继续。
- 不建议在本轮直接承诺完整原生移动端：它涉及平台工具链与媒体权限，风险远高于其他项。

**复杂度**：PWA S；Dioxus Mobile XL；**风险**：高。

### 5.3（可选）私密频道表情反应

**现状**：私密频道表情被显式禁用，避免走主群广播泄露频道 id。

**方案**
- 协议新增 `ServerMessage::ChannelReaction { channel_id, from, ts, ciphertext }`；
- 客户端用频道 key 加密 reaction payload，HTTP/WS 定向 fanout 给频道成员；
- UI 复用现有 reaction pill。
- 复杂度 M；可与批次 2/3 一起做。

---

## 6. 关键决策（需要你确认）

1. **移动端**：默认 PWA 补完；是否额外做 `kaede-mobile` Dioxus 原生 spike？
2. **多设备**：默认“会话管理 + 单设备在线”；是否需要真正多设备同时在线（架构级改动）？
3. **搜索**：默认仅搜索本地已解密历史；是否接受“未缓存历史搜不到”的限制？
4. **SDP/SFU**：默认直接删除并升协议 v5；还是保留 deprecated 变体？
5. **缩略图**：默认客户端生成加密缩略图；还是只做“点击下载原图”不做缩略图？
6. **私密频道表情**：是否纳入本轮？

---

## 7. 交付与验证

每批完成后必须通过：

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server --all-targets -- -D warnings
cargo test -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server
cargo test -p kaede-client --features native
cargo check -p kaede-wasm --target wasm32-unknown-unknown
cargo check -p kaede-client --features wasm --target wasm32-unknown-unknown
```

新增回归至少覆盖：
- 好友实时事件（在线/离线）
- 未读计数与清零
- @ 解析与高亮
- 本地搜索命中/无结果
- 会话列表/撤销
- 附件进度回调（native/wasm）
- 缩略图上传/下载/解密
- 频道成员移除后的 MLS epoch 一致性
- 协议 v5 握手/旧客户端拒绝

**部署顺序**：先服务端（支持 v5）→ 再客户端；旧客户端会被明确拒绝，不会静默解密失败。

---

## 8. 建议的默认选择

如果你没有额外偏好，我建议按以下默认执行：

- 移动端：**PWA 补完**，Dioxus Mobile 只做 spike 评估
- 多设备：**会话管理 + 单设备在线**，不做真正多设备
- 搜索：**本地已解密搜索**
- SDP/SFU：**直接删除，协议 v5**
- 缩略图：**客户端生成加密缩略图**
- 私密频道表情：**本轮不做**（放到下一轮）

确认后我按“批次 1 → 2 → 3 → 4”开始实现，每批完成后跑全量回归并更新报告。
