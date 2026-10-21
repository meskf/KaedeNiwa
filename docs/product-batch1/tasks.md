# 批次 1 任务清单

状态标记：`[ ]` 未开始 / `[x]` 完成

## A. 协议 v5 + 好友实时推送

- [x] A1 `kaede-protocol/src/lib.rs`：`PROTOCOL_VERSION = 5`、`MIN_SUPPORTED_VERSION = 5`、新增 `ServerMessage::FriendsChanged`
- [x] A2 `kaede-protocol` round-trip 测试覆盖 `FriendsChanged`
- [x] A3 `kaede-server/src/friends.rs`：`send_request` / 反向自动接受 / `accept_request` / `remove_friend` 成功后 `send_to` 通知双方
- [x] A4 `kaede-server` 测试：写操作后接收端收到 `FriendsChanged`
- [x] A5 `kaede-client/src/event.rs`：新增 `ClientEvent::FriendsChanged`
- [x] A6 `kaede-client/src/connection.rs`：处理 `ServerMessage::FriendsChanged`
- [x] A7 `kaede-ui/src/observer.rs`：当前 server 匹配时调用 `load_friends_into`

## B. 8K/4K 档位生效

- [x] B1 `kaede-ui/src/video_picker.rs`：移除 Phase 3a/3b 文案；共享中切档自动重建发布
- [x] B2 `kaede-livekit-web/src/wasm_impl.rs`：`set_screen_share` 接收 `VideoProfile` 并设置 `resolution`
- [x] B3 `kaede-livekit-web/src/native_stub.rs`：同步签名
- [x] B4 `kaede-ui/src/voice_web.rs` + `sidebar.rs`：透传当前档位
- [x] B5 `kaede-ui/src/state.rs` 过时注释清理；README 画质条目清理

## C. 频道成员列表 / 退出

- [x] C1 `kaede-server/src/error.rs`：新增 `Forbidden`
- [x] C2 `kaede-server/src/channels.rs`：`GET /api/channels/{id}/members` + 公开/私密权限
- [x] C3 `kaede-server` 测试：公开可看 / 私密非成员 403 / 成员可看
- [x] C4 `kaede-client/src/http.rs`：`ChannelMember` + `list_channel_members`
- [x] C5 `kaede-client/src/room.rs`：包装 `list_channel_members`
- [x] C6 `kaede-ui/src/state.rs`：`channel_members_open`
- [x] C7 `kaede-ui/src/sidebar.rs`：右键“查看成员”、“退出频道”文案、`ChannelMembersModal`
- [x] C8 `kaede-ui/src/app.rs`：挂载成员弹窗

## D. 回归验证

- [x] D1 `cargo fmt --all -- --check`
- [x] D2 `cargo check --workspace --all-targets`
- [x] D3 核心 6 crate clippy `-D warnings`
- [x] D4 核心 6 crate 测试
- [x] D5 `cargo test -p kaede-client --features native`
- [x] D6 `cargo check -p kaede-wasm --target wasm32-unknown-unknown`
- [x] D7 `cargo check -p kaede-client --features wasm --target wasm32-unknown-unknown`
- [x] D8 更新 `docs/product-plan-2026-10-22.md` 批次 1 状态
