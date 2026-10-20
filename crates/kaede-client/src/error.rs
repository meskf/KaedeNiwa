//! 客户端错误。

use crate::state::TransitionError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("mls error: {0}")]
    Mls(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("invalid state transition: {0}")]
    State(#[from] TransitionError),
    #[error("server error ({code:?}): {message}")]
    Server {
        code: kaede_protocol::ErrorCode,
        message: String,
    },
    #[error("not authenticated")]
    NotAuthenticated,
    #[error("not joined")]
    NotJoined,
    #[error("connection closed")]
    Closed,

    #[error("request timed out")]
    Timeout,
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("http error ({status}): {message}")]
    Http { status: u16, message: String },

    /// P1-γ: snapshot 所需的 cipher 在当前平台不可用（典型场景：Windows
    /// 上写入的 DPAPI snapshot 在 Linux/macOS/Web 上打开）。调用方应视为
    /// "本地 MLS 身份丢失，需要重建"——旧加密历史不可恢复，但可以通过
    /// 重新生成身份 + 重新拉群恢复可用性。
    ///
    /// **不是** snapshot 损坏；真正的数据损坏会返回 `Mls`。
    #[error(
        "snapshot incompatible with current platform (alg_id={alg:#04x}); \
             MLS identity will be rebuilt; old encrypted history is not recoverable"
    )]
    SnapshotIncompatible { alg: u8 },
}

impl From<kaede_mls::MlsError> for ClientError {
    fn from(e: kaede_mls::MlsError) -> Self {
        ClientError::Mls(e.to_string())
    }
}

impl From<kaede_storage::StorageError> for ClientError {
    fn from(e: kaede_storage::StorageError) -> Self {
        ClientError::Storage(e.to_string())
    }
}
