//! DM / 私密频道 group 的快照持久化：dirty 跟踪、尾随 flush、启动恢复。
//!
//! 主群快照由 `RoomSession::save_snapshot` 负责（取 `take_main_dirty` 标志）；
//! 这里只负责 DM / 私密频道 / DM 代际元数据三类 aux snapshot。
//! 任何一次落盘失败都会置 `snapshot_broken` 闭锁发送（防 ratchet 回退）。

use crate::error::ClientError;
use crate::event::ClientEvent;

use super::{AuxStorageHook, ServerConnection};

impl ServerConnection {
    /// B3+: 注入 aux storage hook。由 RoomSession 在构造后调用。
    pub fn set_aux_storage(&self, hook: AuxStorageHook) {
        *self.aux_storage.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    /// B3+: 把所有 dirty snapshot flush 到 storage。没有 hook 时 no-op。
    pub async fn flush_dirty_snapshots(&self) {
        let hook = match self
            .aux_storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            Some(h) => h,
            None => return,
        };
        let cipher = crate::keystore::device_cipher(&hook.uid, &hook.server_url);

        // 与 RoomSession::save_snapshot 串行化主群写盘，且锁内重新取快照；
        // 否则并发写同一 key 时旧状态可能后写覆盖新状态。
        let _write_guard = self.lock_snapshot_write().await;

        // 本次落盘是否出现不可忽略的失败。任何失败都意味着内存里的 MLS 状态
        // 比磁盘更新：此时必须闭锁发送，等下一次成功落盘再恢复。
        let mut failures: Vec<String> = Vec::new();

        // 主群状态优先落盘：发送/接收消息都会推进 ratchet，
        // 不保存会导致重启后 generation/nonce 复用。
        if self.take_main_dirty() {
            match self.mls_snapshot_with_cipher(cipher.as_ref()) {
                Ok(snap) => {
                    // 必须与 `RoomSession::save_snapshot` 使用完全相同的 Key，
                    // 否则接收侧推进的 MLS 状态在重启后读不到。
                    let key = format!("mls/{}/{}", hook.server_url, hook.uid);
                    match hook.store.put("mls", &key, &snap).await {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::warn!("[main-persist] put failed: {} (will retry)", e);
                            self.restore_main_dirty();
                            failures.push(format!("main: {e}"));
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("[main-persist] snapshot failed: {}", e);
                    self.restore_main_dirty();
                    failures.push(format!("main: {e}"));
                }
            }
        }

        let dm_ids: Vec<String> = self
            .dirty_dm_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        for conv_id in dm_ids {
            match self.dm_snapshot_with_cipher(&conv_id, cipher.as_ref()) {
                Ok(snap) => {
                    let key = format!("{}/{}", hook.server_url, conv_id);
                    match hook.store.put("dm", &key, &snap).await {
                        Ok(()) => {
                            self.dirty_dm_groups
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&conv_id);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[aux-persist] dm {} put failed: {} (will retry)",
                                conv_id,
                                e
                            );
                            failures.push(format!("dm {conv_id}: {e}"));
                        }
                    }
                }
                // group 已不存在（本地已 drop）：dirty 无意义，直接剔除。
                Err(e) if is_missing_group(&e) => {
                    self.dirty_dm_groups
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&conv_id);
                }
                // 其余失败必须保留 dirty 并闭锁：epoch/ratchet 已前进但没落盘。
                Err(e) => {
                    tracing::warn!("[aux-persist] dm {} snapshot failed: {}", conv_id, e);
                    failures.push(format!("dm {conv_id}: {e}"));
                }
            }
        }

        let ch_ids: Vec<String> = self
            .dirty_channel_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        for channel_id in ch_ids {
            match self.channel_snapshot_with_cipher(&channel_id, cipher.as_ref()) {
                Ok(snap) => {
                    let key = format!("{}/{}", hook.server_url, channel_id);
                    match hook.store.put("channel", &key, &snap).await {
                        Ok(()) => {
                            self.dirty_channel_groups
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&channel_id);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[aux-persist] channel {} put failed: {} (will retry)",
                                channel_id,
                                e
                            );
                            failures.push(format!("channel {channel_id}: {e}"));
                        }
                    }
                }
                Err(e) if is_missing_group(&e) => {
                    self.dirty_channel_groups
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&channel_id);
                }
                Err(e) => {
                    tracing::warn!(
                        "[aux-persist] channel {} snapshot failed: {}",
                        channel_id,
                        e
                    );
                    failures.push(format!("channel {channel_id}: {e}"));
                }
            }
        }

        // DM 代际元数据：用于单侧重建后的仲裁与崩溃恢复。
        let meta_ids: Vec<String> = self
            .dirty_dm_meta
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        for conv_id in meta_ids {
            let generation = self.dm_generation(&conv_id);
            let key = format!("{}/{}", hook.server_url, conv_id);
            match hook
                .store
                .put("dm_meta", &key, &generation.to_be_bytes())
                .await
            {
                Ok(()) => {
                    self.dirty_dm_meta
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&conv_id);
                }
                Err(e) => {
                    tracing::warn!(
                        "[aux-persist] dm_meta {} put failed: {} (will retry)",
                        conv_id,
                        e
                    );
                    failures.push(format!("dm_meta {conv_id}: {e}"));
                }
            }
        }

        if failures.is_empty() {
            self.clear_snapshot_broken();
        } else {
            self.mark_snapshot_broken(&failures.join("; "));
        }
    }

    /// B3+: 从 storage 恢复所有 DM / 私密频道 snapshot。
    ///
    /// 返回需要 emit 到 UI 的 `MlsSyncError` 列表。恢复失败的 group 不会进 map，
    /// 下次 `dm_ensure_group` / `channel_ensure_group` 会重建。
    pub async fn load_aux_snapshots_from_storage(&self, hook: &AuxStorageHook) -> Vec<ClientEvent> {
        let mut errors = Vec::new();
        let prefix = format!("{}/", hook.server_url);

        // 先恢复 DM 代际（必须在 group 恢复前/后都可以，独立命名空间）。
        if let Ok(keys) = hook.store.list_keys("dm_meta").await {
            for key in keys {
                let Some(conv_id) = key.strip_prefix(&prefix) else {
                    continue;
                };
                if let Ok(Some(bytes)) = hook.store.get("dm_meta", &key).await
                    && bytes.len() == 8
                {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(&bytes);
                    let generation = u64::from_be_bytes(buf);
                    self.dm_generations
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(conv_id.to_string(), generation);
                }
            }
        }

        let dm_keys = match hook.store.list_keys("dm").await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("[aux-restore] list dm keys failed: {}", e);
                Vec::new()
            }
        };
        for key in dm_keys {
            let Some(conv_id) = key.strip_prefix(&prefix) else {
                continue;
            };
            let conv_id = conv_id.to_string();
            let snap = match hook.store.get("dm", &key).await {
                Ok(Some(s)) => s,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("[aux-restore] get dm {} failed: {}", key, e);
                    continue;
                }
            };
            let cipher =
                crate::keystore::device_cipher_for_snapshot(&hook.uid, &hook.server_url, &snap);
            match self.dm_restore_from_snapshot_with_cipher(&conv_id, &snap, Some(cipher.as_ref()))
            {
                Ok(()) => {
                    tracing::info!("[aux-restore] dm {} restored", conv_id);
                }
                Err(ClientError::SnapshotIncompatible { alg }) => {
                    tracing::error!(
                        "[aux-restore] dm {} snapshot incompatible (alg={:#04x})",
                        conv_id,
                        alg
                    );
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!(
                            "DM 会话 {} 状态因平台不兼容丢失，请让对方重新发送一条消息",
                            conv_id
                        ),
                        recoverable: true,
                        detail: format!("SnapshotIncompatible alg={:#04x}", alg),
                    });
                }
                Err(e) => {
                    tracing::warn!("[aux-restore] dm {} restore failed: {}", conv_id, e);
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!(
                            "DM 会话 {} 状态恢复失败，请让对方重新发送一条消息",
                            conv_id
                        ),
                        recoverable: true,
                        detail: e.to_string(),
                    });
                }
            }
        }

        let ch_keys = match hook.store.list_keys("channel").await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("[aux-restore] list channel keys failed: {}", e);
                Vec::new()
            }
        };
        for key in ch_keys {
            let Some(channel_id) = key.strip_prefix(&prefix) else {
                continue;
            };
            let channel_id = channel_id.to_string();
            let snap = match hook.store.get("channel", &key).await {
                Ok(Some(s)) => s,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("[aux-restore] get channel {} failed: {}", key, e);
                    continue;
                }
            };
            let cipher =
                crate::keystore::device_cipher_for_snapshot(&hook.uid, &hook.server_url, &snap);
            match self.channel_restore_from_snapshot_with_cipher(
                &channel_id,
                &snap,
                Some(cipher.as_ref()),
            ) {
                Ok(()) => {
                    tracing::info!("[aux-restore] channel {} restored", channel_id);
                }
                Err(ClientError::SnapshotIncompatible { alg }) => {
                    tracing::error!(
                        "[aux-restore] channel {} snapshot incompatible (alg={:#04x})",
                        channel_id,
                        alg
                    );
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!(
                            "私密频道 {} 状态因平台不兼容丢失，请重新加入",
                            channel_id
                        ),
                        recoverable: true,
                        detail: format!("SnapshotIncompatible alg={:#04x}", alg),
                    });
                }
                Err(e) => {
                    tracing::warn!("[aux-restore] channel {} restore failed: {}", channel_id, e);
                    errors.push(ClientEvent::MlsSyncError {
                        server_id: self.server_id.clone(),
                        user_message: format!("私密频道 {} 状态恢复失败，请重新加入", channel_id),
                        recoverable: true,
                        detail: e.to_string(),
                    });
                }
            }
        }

        errors
    }
}

/// snapshot 失败是否只是“本地 group 已不存在”。
///
/// `dm_drop_group` / `channel_drop_group` 之后 dirty 标记可能还在，但内存里
/// 已经没有该 group，没有任何状态需要落盘 —— 这类失败不应闭锁发送。
fn is_missing_group(err: &ClientError) -> bool {
    matches!(err, ClientError::Mls(msg) if msg.contains("not found"))
}
