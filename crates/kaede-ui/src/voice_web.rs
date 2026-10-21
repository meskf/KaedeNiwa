//! wasm32 语音路径：用 `kaede_livekit_web` 直连 LiveKit (浏览器原生 WebRTC)。
//!
//! ## 设计
//! - Session 存 `Rc<LiveKitWebSession>` 于 thread_local。wasm 单线程，Rc 足够且安全。
//! - 每次操作先 `Rc::clone` 出来，跨 `await` 期间即使 `SESSION` 被清空也不影响。
//! - 与 native 分支对等：connect_voice / set_microphone / set_screen_share / leave_voice。
//!
//! ## 已知限制
//! - 远端 track 目前由浏览器自动订阅音频并播放；把远端视频 attach 到 DOM 留到下一期。
//! - DM / 私密频道走 C14：E2EE 关闭（由 `begin_media_session` 返回 `None`）。

use std::cell::RefCell;
use std::rc::Rc;

use kaede_livekit_web::LiveKitWebSession;

use crate::state::{VoiceControls, VoiceState};

thread_local! {
    static SESSION: RefCell<Option<Rc<LiveKitWebSession>>> = const { RefCell::new(None) };
}

/// 断开当前会话（幂等）。Drop 里也会兜底 disconnect，这里是显式清理 + 允许重建。
pub async fn leave_voice() {
    let old = SESSION.with(|s| s.borrow_mut().take());
    if let Some(s) = old {
        if let Err(e) = s.disconnect().await {
            tracing::debug!("[voice-web] disconnect on leave: {}", e);
        }
    }
}

/// 连到某频道媒体房间。失败时不动 `voice.active_channel`，由调用方回滚。
pub async fn connect_voice(
    _voice: VoiceState,
    _controls: VoiceControls,
    room: std::sync::Arc<kaede_client::room::RoomSession>,
    channel_id: String,
) -> Result<(), String> {
    leave_voice().await;

    let (token, key) = room
        .begin_media_session(&channel_id)
        .await
        .map_err(|e| format!("begin_media_session: {}", e))?;

    let key_bytes: Option<[u8; 32]> = key.map(|z| *z);
    let session = LiveKitWebSession::new(key_bytes)
        .await
        .map_err(|e| format!("LiveKitWebSession::new: {}", e))?;

    session
        .connect(&token.ws_url, &token.token)
        .await
        .map_err(|e| format!("connect: {}", e))?;

    session
        .attach_remote_videos_to("kaede-remote-videos")
        .map_err(|e| format!("attach_remote_videos: {}", e))?;

    SESSION.with(|s| *s.borrow_mut() = Some(Rc::new(session)));
    tracing::info!(
        "[voice-web] connected channel={} room={} (e2ee={})",
        channel_id,
        token.room,
        key_bytes.is_some()
    );
    Ok(())
}

fn current_session() -> Result<Rc<LiveKitWebSession>, String> {
    SESSION
        .with(|s| s.borrow().as_ref().cloned())
        .ok_or_else(|| "no active media session".to_string())
}

/// 开关麦克风。
pub async fn set_microphone(enabled: bool) -> Result<(), String> {
    let session = current_session()?;
    session
        .set_microphone(enabled)
        .await
        .map_err(|e| format!("setMicrophoneEnabled: {}", e))
}

/// 开关屏幕共享。`true` 时浏览器弹出 getDisplayMedia 选择器。
pub async fn set_screen_share(enabled: bool) -> Result<(), String> {
    let session = current_session()?;
    session
        .set_screen_share(enabled)
        .await
        .map_err(|e| format!("setScreenShareEnabled: {}", e))
}
