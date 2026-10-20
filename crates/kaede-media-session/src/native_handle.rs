//! Native MediaSessionHandle：包装 [`crate::MediaSession`]。

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::handle::MediaSessionHandle;
use crate::{ConnectConfig, MediaEvent, MediaSession, PublishSource, TrackKind};
use kaede_media::MediaError;

pub struct NativeHandle {
    inner: Arc<MediaSession>,
}

#[async_trait]
impl MediaSessionHandle for NativeHandle {
    async fn connect(cfg: ConnectConfig) -> Result<Arc<Self>, MediaError> {
        let inner = MediaSession::connect(cfg).await?;
        Ok(Arc::new(Self { inner }))
    }

    fn events(&self) -> broadcast::Receiver<MediaEvent> {
        self.inner.events()
    }

    fn local_uid(&self) -> String {
        self.inner.local_uid().to_string()
    }

    fn room_name(&self) -> String {
        self.inner.room_name().to_string()
    }

    async fn publish(&self, source: PublishSource) -> Result<(), MediaError> {
        self.inner.publish(source).await
    }

    async fn unpublish(&self, kind: TrackKind) -> Result<(), MediaError> {
        self.inner.unpublish(kind).await
    }

    async fn disconnect(&self) {
        self.inner.disconnect().await;
    }
}

impl NativeHandle {
    /// 取出内部 MediaSession（用于 attach_video / attach_audio）。
    pub fn inner(&self) -> &Arc<MediaSession> {
        &self.inner
    }
}
