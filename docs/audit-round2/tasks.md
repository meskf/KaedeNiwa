# 第二轮审计修复任务清单

> 依据：`docs/audit-round2/spec.md`
> 状态：已完成（2026-10-21，全部任务通过回归）

## 阶段一：功能阻断与安全边界

- [x] T1 私密频道实时消息（R2-01 / F1）
  - 文件：`crates/kaede-protocol/src/lib.rs`、`crates/kaede-server/src/http.rs`、`crates/kaede-client/src/connection.rs`、`crates/kaede-client/src/event.rs`、`crates/kaede-ui/src/observer.rs`
  - 验收：双客户端私密频道实时收发；刷新不重复；离线补历史。
  - 回归：新增服务端广播测试 + 客户端事件路径测试。

- [x] T2 封闭私密频道主群旁路（R2-02 / R2-03 / F2）
  - 文件：`crates/kaede-client/src/connection.rs`、`crates/kaede-ui/src/observer.rs`、`crates/kaede-ui/src/content.rs`、`crates/kaede-client/src/room.rs`
  - 验收：主群伪造私密频道消息/反应被丢弃；私密频道表情给出明确提示；媒体信令不再主群广播。

- [x] T3 主群 Welcome 重建（R2-04 / F3）
  - 文件：`crates/kaede-client/src/mls_session.rs`、`crates/kaede-client/src/connection.rs`
  - 验收：host 重建群后 guest 自动切群，双向消息可解；重复 Welcome 无副作用。

- [x] T4 DM 代际仲裁（R2-05 / F4）
  - 文件：`crates/kaede-client/src/connection.rs`、`crates/kaede-client/src/room.rs`
  - 验收：单侧重建 gen=2 后双方收敛；并发首次建群仍收敛。

- [x] T5 媒体 seed 生命周期（R2-06 / F5）
  - 文件：`crates/kaede-server/src/state.rs`、`crates/kaede-server/src/livekit.rs`
  - 验收：TTL 到期回收 participant + seed；`/leave` 立即回收；同通话内 seed 一致。

- [x] T6 默认频道启动创建（R2-07 / F6）
  - 文件：`crates/kaede-server/src/lib.rs`、`crates/kaede-server/src/main.rs`
  - 验收：空库启动后存在 `general` 与 `voice`。

## 阶段二：数据卫生与可恢复性

- [x] T7 删除成员孤儿清理（R2-08 / F7）
  - 文件：`crates/kaede-server/src/members.rs`
  - 验收：channel_members / pending_envelopes / dm_conversations 无残留。

- [x] T8 离线权威补投（R2-09 / F8）
  - 文件：`crates/kaede-server/src/channels.rs`、`crates/kaede-server/src/ws.rs`、`crates/kaede-client/src/room.rs`、`crates/kaede-ui/src/observer.rs`
  - 验收：创建者离线时加入请求不丢；上线后自动补发 Welcome。

- [x] T9 P2 快速修复（R2-10 ~ R2-12 / F9）
  - 文件：`crates/kaede-client/src/room.rs`、`crates/kaede-ui/src/content.rs`
  - 验收：私密频道自己的消息不重复；同毫秒本地消息不覆盖；表情失败有提示。

## 阶段三：体验项（仅范围 B）

- [x] T10 附件大文件渲染优化（R2-13）
- [x] T11 DM 历史分页（R2-14）
- [x] T12 私密频道退出入口（R2-15）
- [x] T13 语音成员列表同步（R2-16）
- [x] T14 wasm/画质/SFU/好友实时通知的文档标注或禁用提示（R2-17 ~ R2-20）

## 最终回归

- [x] T15 运行 spec.md 第 6 节全部命令
- [x] T16 确认新增回归测试全部通过，更新 `docs/audit-round2/spec.md` 状态
