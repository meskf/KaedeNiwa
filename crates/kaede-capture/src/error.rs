//! 捕获错误。

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("gstreamer: {0}")]
    GStreamer(String),

    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error("unsupported on this platform")]
    UnsupportedPlatform,

    #[error("capture not started")]
    NotStarted,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl From<anyhow::Error> for CaptureError {
    fn from(e: anyhow::Error) -> Self {
        CaptureError::GStreamer(e.to_string())
    }
}
