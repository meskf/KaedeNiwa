# 批次 1 规格：体验快赢（好友实时推送 / 画质档位生效 / 私密频道成员）

- 日期：2026-10-22
- 状态：待批准
- 上游方案：`docs/product-plan-2026-10-22.md` 第 2 章
- 约束：E2EE 服务端只存密文；协议本轮统一升到 v5；不做多设备同时在线

---

## 1. 协议变更（一次性 v5）

文件：`crates/kaede-protocol/src/lib.rs`

1. `PROTOCOL_VERSION: u16 = 5`，`MIN_SUPPORTED_VERSION: u16 = 5`，更新常量文档注释（v5：新增好友变更事件）。
2. `ServerMessage` 新增变体：

```rust
/// 好友关系发生变化（请求 / 接受 / 删除）。
/// 无业务负载：客户端收到后重新拉取 GET /api/friends。
FriendsChanged,
```

- 服务端离线不补投；登录后由 HTTP 列表兜底。
- 因为 `#[serde(tag = "t")]` 且双方同版本部署，旧客户端会被版本协商拒绝。

验收：
- `cargo test -p kaede-protocol` 通过；
- 服务端收到 v4 Hello 返回 `UnsupportedVersion`（现有逻辑，常量变更即生效）。

---

## 2. 好友请求实时推送

### 2.1 服务端

文件：`crates/kaede-server/src/friends.rs`

- 在 `send_request`、`accept_request`、`remove_friend` 成功写库后，对相关 uid 调用
  `state.send_to(&UserId::new(uid), ServerMessage::FriendsChanged)`：
  - `send_request` 普通插入：通知 `me` 与 `target`；
  - 反向 pending 自动成为好友：通知 `me` 与 `target`；
  - `accept_request`：通知 `me` 与 `from`；
  - `remove_friend`：通知 `me` 与 `target`。
- 发送失败（对方离线）忽略，不写离线信封。

### 2.2 客户端

文件：`crates/kaede-client/src/event.rs`
- `ClientEvent` 新增 `FriendsChanged { server_id: String }`。

文件：`crates/kaede-client/src/connection.rs`
- `ServerMessage` 主 `match` 增加分支：
  `ServerMessage::FriendsChanged => emit(ClientEvent::FriendsChanged { .. })`。

文件：`crates/kaede-ui/src/observer.rs`
- `apply_event` 增加分支：若事件的 `server_id` 等于 `app.current_server`，`spawn` 调用
  `crate::friends_view::load_friends_into(app)`；否则忽略（切服时 `use_effect` 会重拉）。

文件：`crates/kaede-ui/src/friends_view.rs`
- 复用现有 `pub(crate) async fn load_friends_into`，无需改签名。

验收：
- 在线 A→B 发请求，B 的“待接受”1s 内出现；
- B 离线时请求落库，下次登录列表正确；
- 接受/删除同样实时刷新；
- 非当前 server 的事件不会污染 `friends` 缓存。

---

## 3. 8K/4K 档位真正生效

### 3.1 Native

文件：`crates/kaede-ui/src/sidebar.rs`
- 现有屏幕共享已把 `profile.width/height/fps` 与 `bitrate_bps` 传入 `VideoConfig`，保留。
- 8K/4K 档位已在 `kaede-protocol/src/video.rs::ALL_VIDEO_PROFILES` 完整存在（8K 120/60/30、4K 240/120/60/30），不改表。

文件：`crates/kaede-ui/src/video_picker.rs`
- 去掉“Phase 3a/3b 未接入”提示，改为真实生效说明。
- 选中档位或码率时，若 `controls.screen_on == true`：
  - native：取 `voice.handle`，`unpublish(TrackKind::ScreenVideo)` 后按新档位
    `publish(PublishSource::Screen { config })`；失败则 toast 并 `screen_on = false`；
  - wasm：`voice_web::set_screen_share(false)` 后 `set_screen_share(true, new_profile)`；
  - 成功 toast “已按新档位重新共享”。
- 未在共享时仅更新信号并 toast “下次共享生效”。

### 3.2 Web

文件：`crates/kaede-livekit-web/src/wasm_impl.rs`
- `set_screen_share(&self, enabled: bool, profile: VideoProfile) -> Result<(), LiveKitWebError>`；
- 开启时的 `video` 选项增加：

```js
resolution: { width: profile.width, height: profile.height, frameRate: profile.fps }
```

- 由 LiveKit 转成 `getDisplayMedia` 的 `width/height/frameRate` 约束；浏览器不支持时
  实际分辨率回退由浏览器处理，调用失败返回可读错误。

文件：`crates/kaede-livekit-web/src/native_stub.rs`
- 同步签名，忽略 `profile`。

文件：`crates/kaede-ui/src/voice_web.rs`
- `set_screen_share(enabled: bool, profile: kaede_protocol::VideoProfile)`，透传。

文件：`crates/kaede-ui/src/sidebar.rs`
- wasm 调用处传 `*voice.current_profile.read()`。

### 3.3 注释清理

- `crates/kaede-ui/src/state.rs` 中 `current_profile` 的 “Phase 3a：未接入 media 层” 注释改为真实行为说明。
- README 如有“画质档位未接入”，同步删除/改写。

验收：
- 选 4K60 后 native `VideoConfig` 为 3840×2160@60、目标码率按档位；
- Web 选 4K 后 `setScreenShareEnabled` 的 `video.resolution` 为 4K；
- 共享中切档会重建发布；8K 发布失败有 toast 错误。

---

## 4. 私密频道成员列表 / 退出

### 4.1 服务端

文件：`crates/kaede-server/src/error.rs`
- 新增 `Forbidden(String)`，映射 `403 forbidden`（仅本功能使用）。

文件：`crates/kaede-server/src/channels.rs`
- 路由新增 `GET /api/channels/{id}/members`。
- 处理函数 `list_channel_members`：
  1. `authenticate`；
  2. 查 `channels.is_private`，不存在 → `NotFound("channel")`；
  3. 私密频道且当前用户不在 `channel_members` → `Forbidden`；公开频道所有登录用户可看；
  4. 返回按 `joined_at ASC` 排序的 `{ login_id, nickname, joined_at }`；
     `nickname` 用 `members.nickname`，缺失回退 `login_id`。

### 4.2 客户端

文件：`crates/kaede-client/src/http.rs`
- 新增 `ChannelMember { login_id, nickname, joined_at }` 与
  `list_channel_members(&self, channel_id) -> Result<Vec<ChannelMember>, ClientError>`。

文件：`crates/kaede-client/src/room.rs`
- 新增 `list_channel_members(&self, channel_id)`，透传 HTTP。

### 4.3 UI

文件：`crates/kaede-ui/src/state.rs`
- `UiState` 新增 `channel_members_open: Signal<Option<String>>`，初始化 `None`。

文件：`crates/kaede-ui/src/sidebar.rs`
- 右键菜单增加“查看成员”，设置 `channel_members_open = Some(ch_id)`。
- 私密频道的“退出私密频道”改文案为“退出频道”（仍调 `leave_channel` 并从列表移除）。
- 新增 `#[component] pub fn ChannelMembersModal()`：
  - 打开时 `use_future` 调 `room.list_channel_members`，加载态/错误态；
  - 列表展示昵称、@login_id、加入时间（`observer::day_label` + `format_hhmm`）；
  - 关闭按钮与背板点击关闭。

文件：`crates/kaede-ui/src/app.rs`
- 挂载 `ChannelMembersModal {}`（与 `ChannelContextMenu` 同级）。

验收：
- 公开频道任意登录用户可看成员；
- 私密频道非成员请求返回 403；
- 退出后频道从本人列表消失，`channel_members` 删除；
- 成员弹窗展示正确、可关闭。

---

## 5. 验证

每项完成后统一执行：

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server --all-targets -- -D warnings
cargo test -p kaede-protocol -p kaede-crypto -p kaede-mls -p kaede-storage -p kaede-client -p kaede-server
cargo test -p kaede-client --features native
cargo check -p kaede-wasm --target wasm32-unknown-unknown
cargo check -p kaede-client --features wasm --target wasm32-unknown-unknown
```

需要新增测试：
- `kaede-protocol`：`FriendsChanged` CBOR round-trip（加入现有 round-trip 测试）。
- `kaede-server`：`send_request` / `accept_request` / `remove_friend` 触发 `FriendsChanged`
  （用现有 state 测试的 channel 接收）。
- `kaede-server`：频道成员列表公开/私密/非成员三态。

---

## 6. 明确不做（本批次）

- 多设备同时在线
- 频道成员移除（需要 MLS 协调，批次 3）
- 好友事件离线补投（DB 已持久化）
