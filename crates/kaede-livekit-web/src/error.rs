//! kaede-livekit-web 错误类型。

#[derive(Debug, thiserror::Error)]
pub enum LiveKitWebError {
    #[error("LiveKit JS SDK (window.LivekitClient) not loaded;              ensure index.html includes livekit-client UMD script")]
    SdkNotLoaded,
    #[error("not supported on this platform (kaede-livekit-web only works on wasm32)")]
    NotSupported,
    #[error("LiveKit JS call failed: {0}")]
    Js(String),
}
