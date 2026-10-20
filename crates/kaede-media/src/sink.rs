//! 下游抽象。
//!
//! 首批实现只有 [`NullSink`]（测试用）。
//! MP4 / LiveKit / WHIP 在后续阶段补。

use async_trait::async_trait;

use crate::MediaError;

/// 加密后的待发送帧。
#[derive(Debug, Clone)]
pub struct EncryptedFrame {
    /// SFrame 加密后的完整帧（12 头 + AES-GCM ct + 16 tag）。
    pub data: Vec<u8>,
    /// 是否关键帧（从原始 H.264 帧继承）。
    pub is_keyframe: bool,
    /// 显示时间戳（毫秒）。
    pub pts_ms: i64,
}

/// 下游发送器。
#[async_trait]
pub trait MediaSink: Send + Sync + 'static {
    /// 发送一帧。返回 `Err` 会让 `MediaSession` 转 `Failed` 状态。
    async fn send(&self, frame: EncryptedFrame) -> Result<(), MediaError>;

    /// 关闭并释放资源。可重复调用。
    async fn close(&self) -> Result<(), MediaError>;
}

/// 空 sink：丢弃所有帧。测试与基准用。
pub struct NullSink;

#[async_trait]
impl MediaSink for NullSink {
    async fn send(&self, _frame: EncryptedFrame) -> Result<(), MediaError> {
        Ok(())
    }
    async fn close(&self) -> Result<(), MediaError> {
        Ok(())
    }
}
