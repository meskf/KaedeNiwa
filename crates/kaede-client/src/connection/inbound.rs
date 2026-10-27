//! 收消息：`handle_server_message` 的所有 `ServerMessage` 分支处理。
//!
//! MLS 消息在这里自动解密 / 处理握手；私密频道内容只接受 `0x03` 前缀的
//! 频道密文，主群广播里伪造的私密频道消息 / reaction 会被丢弃。

use kaede_protocol::{ClientMessage, ServerMessage};

use crate::error::ClientError;
use crate::event::ClientEvent;

use super::pending::PendingMainWelcome;
use super::{ServerConnection, dm::unpack_dm_welcome, unpack_welcome_payload};

impl ServerConnection {
    /// 处理一条收到的服务器消息。
    ///
    /// MLS 消息在这里自动解密 / 处理握手。
    pub async fn handle_server_message(&self, msg: ServerMessage) -> Result<(), ClientError> {
        match msg {
            ServerMessage::PeerJoined { uid } => {
                // 去重：重连时服务器可能重发 PeerJoined，避免 peers 出现重复
                let is_new = {
                    let mut p = self.peers.lock().unwrap_or_else(|e| e.into_inner());
                    if p.contains(&uid) {
                        false
                    } else {
                        p.push(uid.clone());
                        true
                    }
                };
                if is_new {
                    self.emit(ClientEvent::PeerJoined {
                        server_id: self.server_id.clone(),
                        uid,
                    });
                }
            }
            ServerMessage::PeerLeft { uid } => {
                self.peers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|u| u != &uid);
                self.emit(ClientEvent::PeerLeft {
                    server_id: self.server_id.clone(),
                    uid,
                });
            }
            ServerMessage::Error { code, message } => {
                self.emit(ClientEvent::ServerError {
                    server_id: self.server_id.clone(),
                    code,
                    message: message.clone(),
                });
                if code.is_fatal() {
                    return Err(ClientError::Server { code, message });
                }
            }

            // ---- MLS 握手 ----
            ServerMessage::MlsWelcome { from, payload } => {
                // Round 6（Signal 模型）：收到 Welcome 时不立即入群。
                // 服务器允许任何已加入成员发 Welcome，客户端通过用户确认
                // 决定是否信任。见 PendingWelcome 事件和
                // accept_pending_welcome / reject_pending_welcome 方法。
                let was_ready = self.mls_is_ready();
                let (gid, _welcome) = unpack_welcome_payload(&payload);
                // 已就绪 + group_id 相同 = 服务器重复投递，直接忽略，
                // 不弹确认框（避免每次重连都问一遍）。
                if was_ready && let Some(incoming_gid) = gid {
                    let is_same = {
                        let guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
                        guard
                            .as_ref()
                            .and_then(|s| s.group_id())
                            .map(|cur| cur == incoming_gid)
                            .unwrap_or(false)
                    };
                    if is_same {
                        tracing::debug!(
                            "[client:{}] duplicate welcome from {} (same group_id) ignored",
                            self.server_id,
                            from
                        );
                        return Ok(());
                    }
                }
                // 存 pending（覆盖同 sender 的旧 welcome：同一人重复发，
                // 我们只保留最新一份，避免 pending 无限增长）。
                let sender_key = from.as_str().to_string();
                {
                    let mut map = self
                        .pending_main_welcomes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    // 每个 sender 只保留最新一份；同时设置总量上限，防止
                    // 多账号批量发 Welcome 把内存撑爆（服务端也有限流兜底）。
                    const MAX_PENDING_MAIN_WELCOMES: usize = 16;
                    if !map.contains_key(&sender_key)
                        && map.len() >= MAX_PENDING_MAIN_WELCOMES
                        && let Some(oldest) = map
                            .iter()
                            .min_by_key(|(_, p)| p.received_at)
                            .map(|(k, _)| k.clone())
                    {
                        map.remove(&oldest);
                    }
                    map.insert(
                        sender_key.clone(),
                        PendingMainWelcome {
                            payload,
                            from: from.clone(),
                            is_rebuild: was_ready,
                            received_at: web_time::Instant::now(),
                        },
                    );
                }
                tracing::info!(
                    "[client:{}] pending welcome from {} (is_rebuild={}); awaiting user confirmation",
                    self.server_id,
                    from,
                    was_ready
                );
                self.emit(ClientEvent::PendingWelcome {
                    server_id: self.server_id.clone(),
                    from,
                    is_rebuild: was_ready,
                    sender_login_id: sender_key,
                });
            }

            ServerMessage::MlsCommit {
                from,
                epoch,
                channel_id,
                payload,
            } => {
                if let Some(ch) = channel_id {
                    // 频道 Commit 补发：payload 可能带 0x05 前缀（补发时保留与实时广播一致），
                    // 先剥离再交给 MLS。
                    let commit_bytes: &[u8] = if payload.first() == Some(&0x05) {
                        &payload[1..]
                    } else {
                        &payload[..]
                    };
                    match self.channel_process_commit(&ch, commit_bytes).await {
                        Ok(_) => {
                            tracing::info!(
                                "[client:{}] channel commit backfill for {} epoch {} ok",
                                self.server_id,
                                ch,
                                epoch
                            );
                            if let Some(ep) = self.channel_epoch(&ch) {
                                self.emit(ClientEvent::ChannelEpochChanged {
                                    server_id: self.server_id.clone(),
                                    channel_id: ch.clone(),
                                    epoch: ep,
                                });
                            }
                            // P0-2：drain 已缓冲的连续 Commit。
                            loop {
                                let next = self.channel_epoch(&ch).unwrap_or(0) + 1;
                                let Some(pending) = self.take_pending_channel_commit(&ch, next)
                                else {
                                    break;
                                };
                                if let Err(e) = self.channel_process_commit(&ch, &pending).await {
                                    tracing::warn!(
                                        "[client:{}] drain buffered channel commit epoch {} failed: {}",
                                        self.server_id,
                                        next,
                                        e
                                    );
                                    break;
                                }
                                if let Some(ep) = self.channel_epoch(&ch) {
                                    self.emit(ClientEvent::ChannelEpochChanged {
                                        server_id: self.server_id.clone(),
                                        channel_id: ch.clone(),
                                        epoch: ep,
                                    });
                                }
                            }
                            if self.has_pending_channel_commits(&ch) {
                                let current = self.channel_epoch(&ch).unwrap_or(0);
                                let _ = self.report_channel_epoch(ch.clone(), current).await;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[client:{}] channel commit backfill for {} epoch {} failed: {}",
                                self.server_id,
                                ch,
                                epoch,
                                e
                            );
                        }
                    }
                } else {
                    // 主群 Commit 走原有缓冲/回滚逻辑。
                    self.handle_main_commit(&from, epoch, payload).await;
                }
            }
            ServerMessage::MlsProposal { from: _, payload } => {
                // C-2: 锁内处理、锁外 emit（避免 emit 阻塞锁）
                let before = self.mls_epoch();
                let result = {
                    let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
                    match guard.as_mut() {
                        Some(s) if s.is_ready() => s.process_handshake(&payload).map(|_| ()),
                        _ => Ok(()), // 未 ready 时静默（无 group 状态可破坏）
                    }
                };
                if result.is_ok() {
                    self.mark_main_dirty();
                    self.emit_epoch_if_changed(before);
                }
                if let Err(e) = result {
                    tracing::warn!("[client:{}] handshake failed: {}", self.server_id, e);
                    self.emit_mls_sync_error(e.to_string());
                }
            }

            ServerMessage::DmReceived {
                conv_id,
                from,
                ts,
                ciphertext,
            } => {
                // B2: Welcome envelope (first byte 0x02) -> join DM group, do not emit to UI
                if let Some((generation, welcome)) = unpack_dm_welcome(&ciphertext) {
                    if let Err(e) = self.dm_join_group(&conv_id, welcome, generation) {
                        tracing::warn!(
                            "[client:{}] dm_join_group failed for conv {}: {}",
                            self.server_id,
                            conv_id,
                            e
                        );
                    }
                } else {
                    match self.dm_open(&conv_id, from.as_str(), &ciphertext) {
                        Ok(plaintext) => {
                            self.emit_local_event(ClientEvent::DmReceived {
                                server_id: self.server_id.clone(),
                                conv_id,
                                from,
                                ts,
                                plaintext,
                            });
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[client:{}] DM decrypt failed (conv {}): {}",
                                self.server_id,
                                conv_id,
                                e
                            );
                        }
                    }
                }
            }
            ServerMessage::MlsAppMessage {
                from,
                channel_id,
                payload,
            } => {
                // 【核心修复】真正的 MLS 密文解密
                match self.mls_decrypt(&payload) {
                    Ok(plaintext) => {
                        self.emit_local_event(ClientEvent::AppMessageReceived {
                            from: from.0.clone(),
                            channel_id,
                            payload: plaintext,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("[mls] 解密 MlsAppMessage 失败: {}", e);
                    }
                }
                let plaintext = {
                    let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
                    match guard.as_mut() {
                        Some(s) if s.is_ready() => match s.decrypt_with_sender(&payload) {
                            Ok((pt, mls_sender)) if mls_sender == from.as_str() => Some(pt),
                            Ok((_, mls_sender)) => {
                                // 服务器改写了 from：MLS 密文本身有效，但归属不可信。
                                tracing::warn!(
                                    "[client:{}] sender mismatch: MLS={} server={}; dropping",
                                    self.server_id,
                                    mls_sender,
                                    from
                                );
                                None
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[client:{}] decrypt from {} failed: {}",
                                    self.server_id,
                                    from,
                                    e
                                );
                                None
                            }
                        },
                        _ => None,
                    }
                };
                if let Some(pt) = plaintext {
                    // 先尝试识别媒体信令（MediaMeta），识别成功就不发 AppMessage。
                    // 其它类型（Message / Reaction / Raw）仍走 AppMessage 事件，
                    // 由上层自行 decode（保持既有行为）。
                    use crate::payload::{DecodedPayload, decode};
                    match decode(&pt) {
                        DecodedPayload::MediaMeta(meta) => {
                            self.emit(ClientEvent::MediaMetaReceived {
                                server_id: self.server_id.clone(),
                                from,
                                meta,
                            });
                        }
                        // R2-02: 私密频道内容不得经主群广播进入本地状态。
                        // 任意主群成员都能构造 `c = <私密频道 id>` 的 payload。
                        DecodedPayload::Message(ref p)
                            if self.channel_has_group(&p.c)
                                || self.is_known_private_channel(&p.c) =>
                        {
                            tracing::warn!(
                                "[client:{}] drop main-group message targeting private channel {}",
                                self.server_id,
                                p.c
                            );
                        }
                        DecodedPayload::Reaction(ref r)
                            if self.channel_has_group(&r.c)
                                || self.is_known_private_channel(&r.c) =>
                        {
                            tracing::warn!(
                                "[client:{}] drop main-group reaction targeting private channel {}",
                                self.server_id,
                                r.c
                            );
                        }
                        _ => {
                            self.emit(ClientEvent::AppMessage {
                                server_id: self.server_id.clone(),
                                from,
                                plaintext: pt,
                            });
                        }
                    }
                }
            }

            ServerMessage::ChannelMemberJoined {
                channel_id,
                requester,
                key_package,
            } => {
                if !self.channel_has_group(&channel_id) {
                    tracing::debug!(
                        "[client:{}] channel_member_joined for {} but local has no group; skip",
                        self.server_id,
                        channel_id
                    );
                } else {
                    let kp_bytes: Vec<u8> = {
                        use base64::Engine as _;
                        match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&key_package)
                        {
                            Ok(b) => b,
                            Err(e) => {
                                tracing::warn!("[client:{}] bad kp b64: {}", self.server_id, e);
                                return Ok(());
                            }
                        }
                    };
                    match self.channel_add_member(&channel_id, &kp_bytes, requester.as_str()) {
                        Ok((commit, welcome)) => {
                            let mut payload = Vec::with_capacity(1 + welcome.len());
                            payload.push(0x04u8);
                            payload.extend_from_slice(&welcome);
                            if let Err(e) = self
                                .transport
                                .send(ClientMessage::ChannelSync {
                                    to: requester.clone(),
                                    channel_id: channel_id.clone(),
                                    payload,
                                })
                                .await
                            {
                                tracing::warn!(
                                    "[client:{}] send ChannelSync welcome failed: {}",
                                    self.server_id,
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "[client:{}] sent welcome for {} to {}",
                                    self.server_id,
                                    channel_id,
                                    requester
                                );
                            }

                            // 把 Commit 广播给频道内其他成员，让所有人的 epoch 对齐。
                            // 旧实现漏了这一步，导致既有成员与新成员 key 不一致。
                            if let Err(e) = self
                                .transport
                                .send(ClientMessage::ChannelCommit {
                                    channel_id: channel_id.clone(),
                                    exclude: Some(requester.clone()),
                                    epoch: self.channel_epoch(&channel_id).unwrap_or(0),
                                    payload: commit,
                                })
                                .await
                            {
                                tracing::warn!(
                                    "[client:{}] send ChannelCommit failed for {}: {}",
                                    self.server_id,
                                    channel_id,
                                    e
                                );
                                self.emit(ClientEvent::MlsSyncError {
                                    server_id: self.server_id.clone(),
                                    user_message: format!(
                                        "私密频道 {} 的成员同步失败，请让新成员重新加入或重新登录",
                                        channel_id
                                    ),
                                    recoverable: true,
                                    detail: e.to_string(),
                                });
                            }

                            // ★ 本地 epoch 已推进（authority 自己 add_member），必须通知 JS 层
                            // 刷新 E2EE key。ChannelMemberJoined 不走 ChannelSync 0x05 路径，
                            // 之前的实现只有失败路径 emit，成功路径完全静默。
                            if let Some(epoch) = self.channel_epoch(&channel_id) {
                                self.emit(ClientEvent::ChannelEpochChanged {
                                    server_id: self.server_id.clone(),
                                    channel_id: channel_id.clone(),
                                    epoch,
                                });
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[client:{}] channel_add_member failed for {}: {}",
                                self.server_id,
                                channel_id,
                                e
                            );
                        }
                    }
                }
            }

            ServerMessage::ChannelSync {
                channel_id,
                epoch,
                payload,
            } => {
                if payload.is_empty() {
                    tracing::warn!("[client:{}] empty ChannelSync", self.server_id);
                    return Ok(());
                }
                match payload[0] {
                    0x04 => {
                        #[cfg(target_arch = "wasm32")]
                        web_sys::console::log_1(
                            &format!(
                                "[ChannelSync 0x04] ch={} welcome_len={}",
                                channel_id,
                                payload.len() - 1
                            )
                            .into(),
                        );
                        let welcome = &payload[1..];
                        match self.channel_join_via_welcome(&channel_id, welcome).await {
                            Ok(()) => {
                                tracing::info!(
                                    "[client:{}] joined channel group {} via welcome",
                                    self.server_id,
                                    channel_id
                                );
                                // P0-2：旧 group 的 buffered Commit 已无意义。
                                self.clear_pending_channel_commits(&channel_id);
                                let sender = self
                                    .channel_pending_welcomes
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&channel_id);
                                if let Some(tx) = sender {
                                    let _ = tx.try_send(Ok(welcome.to_vec()));
                                }
                                self.emit(ClientEvent::ChannelJoined {
                                    server_id: self.server_id.clone(),
                                    channel_id,
                                });
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[client:{}] join_via_welcome failed for {}: {}",
                                    self.server_id,
                                    channel_id,
                                    e
                                );
                                self.emit(ClientEvent::MlsSyncError {
                                    server_id: self.server_id.clone(),
                                    user_message: format!(
                                        "私密频道 {channel_id} 的群成员校验失败，已拒绝加入；\
                                         请确认频道成员列表后重试"
                                    ),
                                    recoverable: true,
                                    detail: e.to_string(),
                                });
                                let sender = self
                                    .channel_pending_welcomes
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&channel_id);
                                if let Some(tx) = sender {
                                    let _ = tx.try_send(Err(e.to_string()));
                                }
                            }
                        }
                    }
                    0x05 => {
                        let commit = &payload[1..];
                        let local = self.channel_epoch(&channel_id).unwrap_or(0);
                        // P0-2：先用服务端携带的 epoch 元数据做 gap 检测。
                        match epoch {
                            Some(server_epoch) if server_epoch > local + 1 => {
                                // 缺口：buffer 起来，请求补发。process 留给 backfill 路径处理。
                                if self.buffer_channel_commit(
                                    &channel_id,
                                    server_epoch,
                                    commit.to_vec(),
                                ) {
                                    tracing::info!(
                                        "[client:{}] channel commit gap: local {}, got {}; buffered + request backfill",
                                        self.server_id,
                                        local,
                                        server_epoch
                                    );
                                    let _ =
                                        self.report_channel_epoch(channel_id.clone(), local).await;
                                } else {
                                    tracing::warn!(
                                        "[client:{}] channel commit gap buffer full for {} (local {}, got {})",
                                        self.server_id,
                                        channel_id,
                                        local,
                                        server_epoch
                                    );
                                }
                                return Ok(());
                            }
                            Some(server_epoch) if server_epoch <= local => {
                                // 重复投递或本地已推进到此 epoch，跳过。
                                tracing::debug!(
                                    "[client:{}] channel commit epoch {} <= local {}; skip",
                                    self.server_id,
                                    server_epoch,
                                    local
                                );
                                return Ok(());
                            }
                            _ => {} // epoch == local + 1 或 None：直接 process
                        }
                        match self.channel_process_commit(&channel_id, commit).await {
                            Ok(_) => {
                                tracing::info!(
                                    "[client:{}] processed channel commit for {}",
                                    self.server_id,
                                    channel_id
                                );
                                if let Some(epoch) = self.channel_epoch(&channel_id) {
                                    self.emit(ClientEvent::ChannelEpochChanged {
                                        server_id: self.server_id.clone(),
                                        channel_id: channel_id.clone(),
                                        epoch,
                                    });
                                }
                                // P0-2：drain 已缓冲的连续 Commit。
                                loop {
                                    let next = self.channel_epoch(&channel_id).unwrap_or(0) + 1;
                                    let Some(pending) =
                                        self.take_pending_channel_commit(&channel_id, next)
                                    else {
                                        break;
                                    };
                                    if let Err(e) =
                                        self.channel_process_commit(&channel_id, &pending).await
                                    {
                                        tracing::warn!(
                                            "[client:{}] buffered channel commit epoch {} apply failed: {}",
                                            self.server_id,
                                            next,
                                            e
                                        );
                                        // 应用失败的 Commit 不能再放回（避免死循环）；请求补发。
                                        let current = self.channel_epoch(&channel_id).unwrap_or(0);
                                        let _ = self
                                            .report_channel_epoch(channel_id.clone(), current)
                                            .await;
                                        break;
                                    }
                                    if let Some(epoch) = self.channel_epoch(&channel_id) {
                                        self.emit(ClientEvent::ChannelEpochChanged {
                                            server_id: self.server_id.clone(),
                                            channel_id: channel_id.clone(),
                                            epoch,
                                        });
                                    }
                                }
                                if self.has_pending_channel_commits(&channel_id) {
                                    let current = self.channel_epoch(&channel_id).unwrap_or(0);
                                    let _ = self
                                        .report_channel_epoch(channel_id.clone(), current)
                                        .await;
                                }
                            }
                            Err(e) => {
                                // 无法直接应用（可能是 epoch 乱序但服务端未携带 epoch，
                                // 或 MLS 校验失败）→ 请求 backfill。
                                let current = self.channel_epoch(&channel_id).unwrap_or(0);
                                tracing::warn!(
                                    "[client:{}] channel commit failed for {} at local epoch {}: {}; requesting backfill",
                                    self.server_id,
                                    channel_id,
                                    current,
                                    e
                                );
                                let _ =
                                    self.report_channel_epoch(channel_id.clone(), current).await;
                                self.emit(ClientEvent::MlsSyncError {
                                    server_id: self.server_id.clone(),
                                    user_message: format!(
                                        "私密频道 {channel_id} 的 Commit 应用失败（可能落后），\
                                         已自动请求补发；若仍失败请重新加入该频道"
                                    ),
                                    recoverable: true,
                                    detail: e.to_string(),
                                });
                            }
                        }
                    }
                    tag => tracing::warn!(
                        "[client:{}] unknown ChannelSync tag {:#x} for {}",
                        self.server_id,
                        tag,
                        channel_id
                    ),
                }
            }

            ServerMessage::ChannelMemberLeft { channel_id, member } => {
                if !self.channel_has_group(&channel_id) {
                    tracing::debug!(
                        "[client:{}] channel_member_left for {} but no local group; skip",
                        self.server_id,
                        channel_id
                    );
                    return Ok(());
                }
                match self.channel_remove_member(&channel_id, member.as_str()) {
                    Ok(commit) => {
                        if let Err(e) = self
                            .transport
                            .send(ClientMessage::ChannelCommit {
                                channel_id: channel_id.clone(),
                                exclude: None,
                                epoch: self.channel_epoch(&channel_id).unwrap_or(0),
                                payload: commit,
                            })
                            .await
                        {
                            tracing::warn!(
                                "[client:{}] send ChannelCommit after leave failed: {}",
                                self.server_id,
                                e
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[client:{}] remove channel member {} failed: {}",
                            self.server_id,
                            member,
                            e
                        );
                    }
                }
            }

            ServerMessage::ChannelRemoved { channel_id } => {
                self.channel_drop_group(&channel_id).await;
                self.set_channel_private(&channel_id, false);
                self.emit(ClientEvent::ChannelDeleted {
                    server_id: self.server_id.clone(),
                    channel_id,
                });
            }

            ServerMessage::ChannelReaction {
                channel_id,
                from,
                ciphertext,
            } => {
                if ciphertext.first() != Some(&0x03) {
                    tracing::warn!(
                        "[client:{}] channel reaction without 0x03 prefix for {}; drop",
                        self.server_id,
                        channel_id
                    );
                } else if !self.channel_has_group(&channel_id) {
                    tracing::warn!(
                        "[client:{}] channel reaction for {} but no local channel group; drop",
                        self.server_id,
                        channel_id
                    );
                } else {
                    match self.channel_open(&channel_id, from.as_str(), &ciphertext[1..]) {
                        Ok(plaintext) => {
                            self.emit(ClientEvent::ChannelReaction {
                                server_id: self.server_id.clone(),
                                channel_id,
                                from,
                                plaintext,
                            });
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[client:{}] channel reaction decrypt failed for {}: {}",
                                self.server_id,
                                channel_id,
                                e
                            );
                        }
                    }
                }
            }

            ServerMessage::ChannelMessage {
                channel_id,
                from,
                ts,
                ciphertext,
            } => {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(
                    &format!(
                        "[ChannelMessage] ch={} from={} has_03={} has_group={} ct_len={}",
                        channel_id,
                        from,
                        ciphertext.first() == Some(&0x03),
                        self.channel_has_group(&channel_id),
                        ciphertext.len()
                    )
                    .into(),
                );

                if ciphertext.first() == Some(&0x03) {
                    // 只接受本地确实持有频道 group 的频道。
                    if !self.channel_has_group(&channel_id) {
                        tracing::warn!(
                            "[client:{}] channel message for {} but no local channel group; drop",
                            self.server_id,
                            channel_id
                        );
                    } else {
                        match self.channel_open(&channel_id, from.as_str(), &ciphertext[1..]) {
                            Ok(plaintext) => {
                                self.emit(ClientEvent::ChannelMessage {
                                    server_id: self.server_id.clone(),
                                    channel_id,
                                    from,
                                    ts,
                                    plaintext,
                                });
                            }
                            Err(e) => {
                                #[cfg(target_arch = "wasm32")]
                                web_sys::console::warn_1(
                                    &format!(
                                        "[ChannelMessage] decrypt FAILED ch={} from={}: {}",
                                        channel_id, from, e
                                    )
                                    .into(),
                                );
                                tracing::warn!(
                                    "[client:{}] channel message decrypt failed for {}: {}",
                                    self.server_id,
                                    channel_id,
                                    e
                                );
                            }
                        }
                    }
                } else {
                    tracing::warn!(
                        "[client:{}] channel message without 0x03 prefix for {}; drop",
                        self.server_id,
                        channel_id
                    );
                }
            }

            ServerMessage::ChannelCreated { channel } => {
                self.emit(ClientEvent::ChannelCreated {
                    server_id: self.server_id.clone(),
                    channel,
                });
            }
            ServerMessage::ChannelUpdated { channel } => {
                self.emit(ClientEvent::ChannelUpdated {
                    server_id: self.server_id.clone(),
                    channel,
                });
            }
            ServerMessage::ChannelDeleted { id } => {
                self.channel_drop_group(&id).await;
                self.set_channel_private(&id, false);
                self.emit(ClientEvent::ChannelDeleted {
                    server_id: self.server_id.clone(),
                    channel_id: id,
                });
            }
            ServerMessage::PresenceUpdated { presence } => {
                self.emit(ClientEvent::PresenceUpdated {
                    server_id: self.server_id.clone(),
                    presence,
                });
            }
            ServerMessage::MemberUpdated { member } => {
                self.emit(ClientEvent::MemberUpdated {
                    server_id: self.server_id.clone(),
                    member,
                });
            }
            ServerMessage::MemberRemoved { login_id } => {
                self.emit(ClientEvent::MemberRemoved {
                    server_id: self.server_id.clone(),
                    login_id,
                });
            }
            ServerMessage::FriendsChanged => {
                self.emit(ClientEvent::FriendsChanged {
                    server_id: self.server_id.clone(),
                });
            }

            // v7：输入中指示。服务端已 exclude 发送者，无需自滤。
            ServerMessage::Typing { channel_id, uid } => {
                self.emit(ClientEvent::Typing {
                    server_id: self.server_id.clone(),
                    channel_id,
                    from: uid,
                });
            }

            ServerMessage::Joined { peers } => {
                *self.peers.lock().unwrap_or_else(|e| e.into_inner()) = peers.clone();
                self.emit(ClientEvent::Joined {
                    server_id: self.server_id.clone(),
                    peers,
                });
            }

            // 其他类型：暂不处理
            ServerMessage::KeyPackageFor { .. } => {}
            ServerMessage::Pong { .. } => {
                self.record_pong();
            }
            ServerMessage::ServerHello { .. } => {}
        }
        Ok(())
    }

    fn emit_mls_sync_error(&self, detail: String) {
        self.emit(ClientEvent::MlsSyncError {
            server_id: self.server_id.clone(),
            user_message: "MLS 同步失败，语音媒体可能中断；如遇异常请离开语音频道重进".into(),
            recoverable: true,
            detail,
        });
    }

    /// 应用一个 epoch 连续的 Commit。
    fn apply_main_commit(&self, epoch: u64, payload: &[u8]) -> Result<(), ClientError> {
        let before = self.mls_epoch();
        {
            let mut guard = self.mls.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_mut() {
                Some(s) if s.is_ready() => s.process_handshake(payload).map(|_| ())?,
                _ => return Ok(()), // 未 ready 时静默
            }
        }
        self.mark_main_dirty();
        self.emit_epoch_if_changed(before);
        let after = self.mls_epoch();
        if after != epoch {
            // Proposal-only / 已被合并的 Commit 可能不推进 epoch，不算错误。
            tracing::debug!(
                "[client:{}] commit epoch {} applied, local epoch now {}",
                self.server_id,
                epoch,
                after
            );
        }
        Ok(())
    }

    /// 处理主群 Commit：按 epoch 排序、缓冲缺口、处理并发提交冲突。
    async fn handle_main_commit(
        &self,
        from: &kaede_protocol::UserId,
        epoch: u64,
        payload: Vec<u8>,
    ) {
        if !self.mls_is_ready() {
            return;
        }
        let Ok(me) = self.self_uid() else {
            return;
        };
        let mut current = self.mls_epoch();

        if epoch <= current {
            // 本地刚提交的 epoch 收到**别人**的 Commit = 本地提交落败。
            // 服务端同 epoch 只保留第一份；必须回滚本地分支再跟随胜出者。
            let losing_local_commit = epoch == current
                && self.commit_backup_epoch() == Some(epoch)
                && from.as_str() != me.as_str();
            if !losing_local_commit {
                tracing::debug!(
                    "[client:{}] stale commit epoch {} (local {}), ignored",
                    self.server_id,
                    epoch,
                    current
                );
                return;
            }
            if let Err(e) = self.rollback_commit_backup() {
                tracing::error!(
                    "[client:{}] rollback losing commit at epoch {} failed: {}",
                    self.server_id,
                    epoch,
                    e
                );
                self.emit_mls_sync_error(format!("回滚本地 MLS Commit 失败：{e}"));
                return;
            }
            current = self.mls_epoch();
            tracing::info!(
                "[client:{}] losing commit epoch {} rolled back; following {}",
                self.server_id,
                epoch,
                from
            );
        }

        if epoch > current + 1 {
            if self.buffer_pending_commit(epoch, payload) {
                tracing::info!(
                    "[client:{}] commit gap: local {}, got {}; buffered and requesting backfill",
                    self.server_id,
                    current,
                    epoch
                );
                self.request_epoch_backfill(current).await;
            }
            return;
        }

        if let Err(e) = self.apply_main_commit(epoch, &payload) {
            tracing::warn!(
                "[client:{}] commit epoch {} apply failed: {}",
                self.server_id,
                epoch,
                e
            );
            // 只有「恢复旧身份后刚重建群」这一种情况可以静默：服务端可能
            // 还残留上一次群的 Commit，它必然无法应用到新群。全新身份建群
            // 时不存在这种残留，任何 Commit 应用失败都必须上报。
            let stale_after_identity_restore = self.identity_restored() && self.mls_epoch() <= 1;
            if !stale_after_identity_restore {
                self.emit_mls_sync_error(e.to_string());
            }
            // 不要在这里再次 ReportEpoch：服务端存的就是这份无法应用的
            // Commit，重复上报只会形成无限重试循环。
            return;
        }
        if self
            .commit_backup_epoch()
            .is_some_and(|e| e <= self.mls_epoch())
        {
            self.discard_commit_backup();
        }

        // 补放之前因缺口缓冲的连续 Commit。
        loop {
            let next = self.mls_epoch() + 1;
            let Some(pending) = self.take_pending_commit(next) else {
                break;
            };
            if let Err(e) = self.apply_main_commit(next, &pending) {
                tracing::warn!(
                    "[client:{}] buffered commit epoch {} apply failed: {}",
                    self.server_id,
                    next,
                    e
                );
                self.emit_mls_sync_error(e.to_string());
                break;
            }
            if self
                .commit_backup_epoch()
                .is_some_and(|e| e <= self.mls_epoch())
            {
                self.discard_commit_backup();
            }
        }

        if self.has_pending_commits() {
            // 仍有缺口：主动补拉（带 5s 退避，避免反复拉同一份坏数据）。
            self.request_epoch_backfill(self.mls_epoch()).await;
        }
    }
}
