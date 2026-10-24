# KaedeNiwa 安全模型与边界

本文件**诚实说明**当前实现的安全边界——哪些场景我们保护、哪些不保护。
每次安全相关的审查（如 2026-10-19 全量审查）都会更新此文件。

## 威胁模型

| 攻击者能力 | 是否防御 | 说明 |
|---|---|---|
| 服务器运营方（可读 DB、可改代码） | **部分** | 不能读明文（MLS 半信任模型），但能看到元数据、可拒绝投递 |
| 网络中间人 | **是** | TLS + MLS/SFrame 双重加密 |
| 拿到本地存储文件（SQLite / IndexedDB） | **Desktop: 是 · Web: 否** | 见下文「快照加密」 |
| 浏览器扩展 / XSS | **否** | Web 端 key 由公开信息派生 |
| 物理访问 | **否** | 无 anti-tamper |
| 量子计算 | **否** | 当前 X25519，未启用 PQ hybrid |

## MLS 群聊

- **encryption**：群内消息走 MLS（RFC 9420），成员变更触发 epoch 前进
- **server 可见**：谁在哪个频道、谁加了谁、消息密文长度、发送时间
- **server 不可见**：消息明文、媒体明文、DM 明文
- **server 可做**：拒绝投递、丢弃消息、投毒 KeyPackage（制造中间人加入）、拒绝新成员加入
- **server 不可做**：读明文、伪造合法 MLS 消息、解密媒体

> 这是**所有基于 MLS 的群聊共有的半信任模型**。Signal 群、Wire、Element
> 都是同样边界。若需要更强的 server 不可信保证，需要引入透明度日志
> （Key Transparency / CONIKS），当前未实现。

## DM（1v1 私聊，B2 方案 · 2026-10-21 升级）
- **encryption**：per-conversation AEAD key = `MLS exporter("kaede-sframe-v1", conv_id || 0x00 || "dm", 32)`
- **隔离级别**：DM 与群频道、与其他 DM、与媒体 key **均独立**
- **✅ E-1 已修复（2026-10-21，B2 方案）**：
  - 每对好友独立的 2 人 MLS group（`dm_groups: HashMap<conv_id, MlsSession>`）
  - 主群成员即使拿到主群 state，**也无法推导** Alice↔Bob 的 DM key
  - Welcome 通过 DM 消息通道交换（首字节 `0x02` 标记）
  - 已由 `room_e2e` / `channel_b2_e2e` 覆盖
- **server 可见**：谁和谁在 DM、消息密文长度、发送时间
- **server 不可见**：DM 明文（除非 server 恶意投毒 KP 加入群）

## 私密频道（B2 方案 · 2026-10-21 新增）

- **准入层**：服务端 `channel_members` 表 + argon2 密码 hash；未授权者无法 list / 发消息 / 读历史
- **加密层**：每个私密频道独立的 MLS group，与主群 / DM / 其它频道均独立
- **加入流程**：
  1. Bob 提交自己的 KeyPackage + 频道密码
  2. 服务端校验密码、写入 `channel_members`
  3. 服务端向已有频道成员广播 `ChannelMemberJoined { channel_id, requester, key_package }`
  4. 已有成员执行 `add_member(bob_kp)` → 得到 Welcome → `ChannelSync { to: bob, payload: [0x04]||welcome }`
  5. Bob 收到 Welcome → `join_with_welcome` → 拿到频道 group key
- **密码学隔离**：非成员即使拿到 `messages` 表 ciphertext（首字节 `0x03` 标记），也没有 channel group key，**无法解密**
- **准入泄露防护**：非成员读不存在频道返回 404 而非 401，避免频道存在性泄露
- **已知限制**：
  - 加入时需要至少一个已有的**权威成员**在线执行 add；Welcome/Commit 本身由服务端
    离线暂存并在对方下次 Join 时补投
  - 成员离开时由权威成员执行 MLS Remove 并广播 Commit，频道密钥轮换
- **验证**：`channel_b2_e2e.rs` 3 个用例全绿（完整流程 / 错密码 / 密码学隔离）

## 媒体 E2EE
- **走 LiveKit 内建 E2EE**（AES-GCM，PBKDF2 从 MLS exporter 派生的 key）
- **key 来源**：`RoomSession::begin_media_session`（MLS exporter + 房间级 seed）
  - **不再**每次进频道都 `self_update`（否则先进/后进的人 epoch 不同，多人语音必然
    互相解不开）
  - 同一次媒体会话内所有参与者共享 server seed → key 一致
  - 跨会话 key 轮换由 server seed 生命周期保证：最后一个参与者调用
    `/api/livekit/leave` 后 seed 被删除，下次进入生成新 seed
  - 崩溃/断网的参与者不会调用 `/leave`。服务端提供
    `POST /api/livekit/webhook`，按 LiveKit 的 HS256 JWT + `sha256(body)`
    校验后处理 `participant_left` / `room_finished`，立即回收 seed；
    未配置 webhook 时只能依赖 8 小时 TTL，复用窗口会相应变长
  - **主动吊销（2026-10）**：只删 DB 行 / MLS 成员不会中断已建立的 LiveKit
    连接（token 6 小时有效）。因此以下操作会同步回收房间 seed，并调用
    LiveKit RoomService `RemoveParticipant` 主动断开：
    - 私密频道移除成员 / 成员主动退出 / 频道被删除
    - 踢出服务器成员、封禁账号（回收该 uid 在所有房间的 seed）
    - 调用失败只记 warn：seed 已回收，被吊销者拿不到新的 e2ee key
  - **保证（配置 webhook 后）**：即使应用重启、LiveKit SDK ratchet 归零，
    两次会话 key 也不同 → **无 nonce 复用**
  - **已知限制**：通话中发生成员变更（epoch 前进）不会热更新已连接会话，需要离开重进
- **server 可见**：谁在哪个语音房间、track 数量、码率
- **server 不可见**：媒体明文、媒体明文加密 key
- **⚠️ 历史遗留（E-3）**：`/api/livekit/e2ee-key` HTTP 端点**已返回 410 Gone**（2026-10-19
  P0-Z 修复）。server 端确定性 key 派生的路径已下线。

### e2ee_seed 与媒体密钥（P0-β / E-3 扩展）

**机制**（2026-10 更新）：媒体 E2EE key 是以下输入的函数：

- `channel_id`（LiveKit room name 的派生）
- `purpose = "livekit-e2ee"`（域分离）
- `e2ee_seed`（32 字节，server 内存生成，每个 LiveKit room 一个）
- MLS group state（由 openmls `export_secret` 派生，client 签名私钥参与）

`e2ee_seed` 由 server `/api/livekit/token` 端点在首次请求 token 时生成，存 server 内存
（`AppState::room_media_seeds`），之后同房间所有客户端拿同一个 seed。客户端用
`MLS exporter(channel_id || 0x00 || "livekit-e2ee" || 0x00 || e2ee_seed)` 派生真正的
AES key。

**这引入了 server 侧的新攻击面**：

- server 内存泄漏 → seed 泄露
- 恶意 server 已知 seed
- **但是**：单纯知道 seed **不足以**推导 key——还需要 MLS group state，它由 client
  私钥派生的 secret 组成，server 端 `key_packages` 表存储的是**公开**数据（不是 secret）

**威胁模型边界（与 E-3 一致）**：

- server 可以做：拒绝投递、投毒 KP 加入群、读取 seed
- server **仍不能**：直接解密媒体（需要同时攻破 MLS 群状态）
- **但 server 内存被完整 dump 时**：连同 seed + in-memory group state 一并泄露 → **可能解密**
- **且恶意 server 可以**：投毒 KP 加入群 → 成为 group member → 能导出同 key → 解密

**用户指引**：

- 与 E-3 相同——server 运营方是**半信任**，不能防范运营方内部的恶意行为
- 与 Signal / Wire / Element 的 server 威胁模型**基本一致**（都是"半信任 server"）
- "完全不可信 server" 需要透明度日志（CONIKS / Key Transparency），**当前未实现**
### 服务器根信任问题（E-3）

即使走 MLS exporter，**e2ee_key 派生自 MLS group state，而 MLS group state 由 server 存储
（`key_packages` 表）**。server 虽然不能直接解密，但**可以选择性投递 KeyPackage 制造中间人加入**。

这是所有基于 MLS 的群聊共有的半信任模型。**用户应理解**：server 可以阻止你加入群、
可以拒绝投递消息，但不能读明文。若需要"server 完全不可信"，需要更复杂的设计
（端到端身份验证 + 透明度日志），**当前未实现**。

### 内存清零边界（S-4，2026-10 新增）

`zeroize` 应用的边界：

| 对象 | 是否 drop 时清零 | 说明 |
|---|---|---|
| `SframeKey` | ✅ | `#[derive(ZeroizeOnDrop)]` |
| `AesGcmCipher.key` | ✅ | `#[derive(ZeroizeOnDrop)]`；`from_bytes` 内部也清零原参数 |
| `MlsIdentity.private`（签名私钥裸字节） | ✅ | `Zeroizing<Vec<u8>>` |
| MLS snapshot 序列化中间值 | ✅ | `snapshot_with` 显式 `zeroize()` |
| exporter 派生 secret（`export_sframe_key_for*`） | ✅ | 拷贝入 `[u8; 32]` 后 `zeroize()` 原 Vec |
| `Aes256Gcm` 内部 key schedule | ✅ | `aes-gcm v0.10.3` 依赖链默认启用 `aes v0.8.4` 的 `zeroize` |
| **`MlsIdentity.signature_keys`（`SignatureKeyPair`）** | ❌ | openmls 0.9 未实现 `Zeroize`；crate 加了 `#![forbid(unsafe_code)]` 无法手写清零。缓解：进程内存 dump 属"完全失陷"场景，超出当前威胁模型；未来随 openmls 升级修复 |
| 局部栈上的临时 `[u8; 32]` 副本 | 🟡 | 编译器控制，无法保证清零（例如 `derive_device_key` 返回值按值传递中间态） |

**已知不可保证**：
- 进程被 dump 时，`SignatureKeyPair` 内部私钥可能在内存中残留
- 局部栈变量的编译器优化可能让 `zeroize` 的写入被消除（极端情况）
- 内核 swap / crash dump / GPU 显存 —— 均不在本项目威胁模型内

以上限制均**不影响协议层安全性**（线协议、存储、密钥派生），仅影响"进程内存被攻破"这一最坏情况下的残留窗口。
## 快照加密（`kaede-client::keystore`）

MLS snapshot 里含**签名私钥**。明文存储等于身份泄漏。各平台策略：

| 平台 | 方案 | 防御范围 |
|---|---|---|
| **Windows Desktop** | DPAPI（`CryptProtectData`） | ✅ 与当前用户账户绑定；换用户/换机器解不开 |
| **macOS Desktop** | Keychain 保存随机 32 字节 key | ✅ 与当前用户钥匙串绑定；钥匙串不可用时拒绝持久化 |
| **Linux Desktop** | Secret Service（GNOME Keyring / KWallet）保存随机 key | ✅ 与当前用户密钥环绑定；不可用时拒绝持久化并暂停发送 |
| **Web（浏览器）** | SHA256(公开信息) + AES-256-GCM | ❌ 任何能读 IndexedDB 的攻击者都能解密 |
| **Android / iOS（Dioxus mobile）** | 同 Web fallback（SHA256 + AES-GCM），文件位于 App 私有沙箱 | ❌ 能执行 App 内代码/读取沙箱者可解密；不防越狱/root |

### Web 端的安全性 ≈ 0（S-2）

Web fallback 的 key 来自 `uid` + `server_url`——**都是公开信息**。任何拿到 IndexedDB
的攻击者（浏览器扩展、XSS、物理访问、恶意本地程序）都能**用公开信息重算 key → 解密
snapshot → 拿到签名私钥**。

**生产建议**：
- Web 端**只用于低价值账号**，不要承载生产身份
- 生产部署优先 Desktop（Windows DPAPI / macOS Keychain / Linux Secret Service）
- Linux Desktop 构建需要 `libdbus-1-dev` + `pkg-config`；无 Secret Service 的环境（如无头服务器）
  不会退回公开信息派生 key，而是拒绝保存快照并暂停发送
- 移动端当前**未接入** Android Keystore / iOS Keychain；内测阶段只适合低价值账号。
  升级路径：`android-keystore` + `security-framework`（iOS）保存随机 DEK，再把
  snapshot 外层头升级为 `0x04`（与 Web DEK 迁移一起做）。
- 未来升级路径：WebCrypto `generateKey(non-extractable)` + IndexedDB 存 handle
  - 必须作为 **snapshot 格式升级**（`0x03` → `0x04`）一起做
  - 与 S-1 的 KDF 修正一起做
  - **当前不做**

### 已知历史 bug（S-1）

`keystore::fallback::derive_device_key` 的 `server_url` 被 hash 了**两次**（copy-paste 错误）。
**不能单独修**——修会让所有现有 Web 用户丢失本地身份。必须与 snapshot 格式升级一起做。
详见 `fallback::derive_device_key` 的 doc comment。

## 管理后台

- **独立 listener**：默认 `127.0.0.1:18081`，与用户 API / WS 端口分离；只在
  `KAEDE_ADMIN_ENABLED=1` 时启动。
- **认证**：`X-Admin-Token`，与用户 session token 完全隔离；服务端对 token 做 SHA-256
  定长比较。`KAEDE_ADMIN_TOKEN` 为空时后台拒绝所有请求（fail-closed）。
- **边界**：后台只能看账号/频道/消息**元数据**与内存日志，不能解密任何消息；
  `/api/logs` 不会返回启动时打印的 Admin Token。
- **部署**：远程访问应保持 loopback 绑定 + SSH 隧道，或使用 HTTPS 反向代理；
  明文 HTTP + 公网绑定会泄露 Admin Token。

## 多设备

- **当前**：**单设备登录**（后登踢前登），已实现 `Kicked` 错误
  - 2026-10-21 修复：`Join` 的幂等判断改为 session-aware；不同连接的 `Join`
    会真正顶替旧连接（旧实现会让新连接变成无法发消息的僵尸）
- **会话 token**：DB 只存 SHA-256(token)，泄漏 DB 不能直接登录
- **MLS 状态持久化**：主群快照在每次发送/接收消息、处理 commit 后落盘
  - 不落盘会导致重启后 sender ratchet 回退 → MLS 层 AES-GCM nonce 复用
- **历史可读性**：`max_past_epochs = 64`（换取可回放历史，削弱前向保密）
  - 仅对**当时在群里的成员**有效；后加入的成员无法解密旧 epoch（MLS 语义）
  - 自己的消息由客户端本地明文缓存（MLS 不允许解密自己的 Application Message）
- **不支持**：同一账号同时在线多设备（MLS state 同步、私钥共享未设计）
- **升级路径（E-2）**：1-3 个月工作量
  - 需要 device pairing 协议（QR code / 6 位码 / 扫码）
  - 需要 MLS key share（设备间安全交换）
  - 需要 state 增量同步协议

## 不防御的威胁

- **本机被完全攻陷**（恶意进程、rootkit、管理员权限）：任何客户端都无法防御
- **服务器与客户端被同时攻陷**：MLS 半信任模型的边界
- **量子计算**：当前 ciphersuite 是 X25519，未走 PQ hybrid。MLS 规范有 PQ 扩展，当前未启用
- **侧信道**（时序、功耗）：未做抗侧信道设计
- **供应链攻击**（依赖被投毒）：Cargo.lock 存在但无签名验证
- **Server DoS**：有 rate limit，但没有完整的抗 DDoS（需要反向代理层）

## 审查历史

- **2026-10-19**：全量代码审查（约 20k 行 Rust）。发现 P0×3 / P1-Critical×10 / P1-Security×10 / P2×45。
  本次修复 P0×3 + P1-Critical×10 + P1-Security×8（S-1/S-2 文档化）。详见 git log 里的 `fix(security)` 提交。
## 审查历史补充

### 2026-10-20（R6）：kaede-crypto 全审

**范围**：`aead.rs` / `collision.rs` / `dm_cipher.rs` / `hkdf.rs` / `sframe.rs` / `lib.rs`

**结论**：**密码学实现正确**。RFC 5869 测试向量通过，SFrame nonce 构造正确，`find_sender_id_collision` 的生日悖论检测覆盖到位。

**已修问题**（P1，不阻断生产）：
- `sframe::decrypt` 裸 API 无 sender 白名单 / 重放防护 → 标记 `#[deprecated]`（见下）
- 上层应始终使用 `kaede-media::receiver::ReceiverSession::decrypt`，它已有：
  - `expected_sender_id: Option<u32>`（`with_sender` 设置）
  - `replay_by_sender`：每个 sender 64 帧滑动窗口，窗口内允许乱序、拒绝重复与窗口外旧帧

**未修复（P2 / 未来）**：
- `expected_sender_id: Option<u32>` 只支持单个 sender → 多人通话时调用方只能设 `None` 放弃防护。若未来应用层 SFrame 路径启用，需改为 `HashSet<u32>` + `with_senders(...)`
- `dm_cipher::open` 的 `ver` 字段不在 AAD 内（当前只有 `0x01` 版本，实际安全；未来加版本时需迁移）

**威胁模型边界**：
- **生产路径（`kaede-media-session` → LiveKit E2EE）不走 `kaede-crypto::sframe`**
- **`kaede-media` 的 SFrame 路径是"未来能力"**（自建 SFU / 离线落盘）
- 未来启用时，调用方 MUST 用 `ReceiverSession::decrypt` + `with_senders(...)`
- `sframe::decrypt` 的 `#[deprecated]` 是**编译期强制**——新代码无法绕过

## 2026-10-23 全量审查收口

本次审查覆盖 15 个 crate（约 4.8 万行 Rust）与当前未提交工作区，修复了以下安全/一致性边界：

### 快照恢复 fail-closed

- `MlsSession::from_snapshot_with` 只接受 `0x03` 外层头 + 非 `ALG_NOOP` 的加密快照；
  明文旧格式直接拒绝并重建身份。
- Web 端 `init_web_keystore` 失败会阻断登录；`device_cipher` 在 DEK 不可用时返回
  不可写 cipher，**不再回退**到由公开信息派生的 key。
- **仍然存在**：本地写权限攻击者可以回滚旧密文（无 TPM/单调计数器）；
  平台钥匙串保护的是机密性，不是防回滚。

### MLS Commit 并发仲裁

- 服务端同一 epoch 只入库并广播第一份 Commit；落败 Commit 不再广播。
- 客户端按 epoch 排序：超前 Commit 暂存并 `ReportEpoch` 补拉，落后/重复丢弃；
  本地刚生成但落败的 Commit 用内存回滚备份恢复到提交前状态，再跟随胜出者。
- 回滚备份只存在内存（`Zeroizing`），不写盘。

### 资源上限

- 频道 / DM 历史接口单次响应密文总量上限 8 MiB，并增加按 uid 的历史读取限流。
- 频道密码校验的 Argon2 并发额度改为按**存储档位**计费（256 MiB 档 = 4 份）。
- `Typing` 扇出计入房间广播预算；WS 增加单 IP 并发连接上限（64），
  未认证握手超时从 10s 缩短到 5s。
- `KAEDE_TRUSTED_PROXIES` 任一条目非法时启动失败（不再静默丢弃）；compose
  固定 `kaede-net` 为 `172.28.0.0/16`，只信任该网段。

### SFrame 重放窗口

- 接收端从“seq 必须严格递增”改为每 sender 64 帧滑动窗口：
  允许 RTP 抖动造成的窗口内乱序，重复帧与窗口外旧帧仍被拒绝。


