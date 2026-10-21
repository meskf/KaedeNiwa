//! MLS 层错误。

#[derive(Debug, thiserror::Error)]
pub enum MlsError {
    #[error("openmls error: {0}")]
    OpenMls(String),
    #[error("tls codec error: {0}")]
    TlsCodec(String),
    #[error("group not found: {0}")]
    GroupNotFound(String),
    #[error("not an application message")]
    NotApplicationMessage,
    #[error("storage error: {0}")]
    Storage(#[from] kaede_storage::StorageError),
    /// P0-A: Sender ID collision. SFrame derives the high 32 bits of its
    /// counter from SHA256(uid)[0..4]; collision means two senders share
    /// the nonce space and break AES-GCM. Rejecting the member is the
    /// only correct mitigation.
    #[error("sender_id collision: candidate={candidate} existing={existing}")]
    SenderIdCollision { candidate: String, existing: String },
    /// C8/H8: KeyPackage 的 credential 与调用方期望的 uid 不一致。
    /// 这是身份冒充的防线：MLS 安全依赖 credential 准确标识成员，
    /// 若 KP 可伪造 credential，则群成员无法区分真假身份。
    #[error("credential mismatch: expected={expected} got={got}")]
    CredentialMismatch { expected: String, got: String },
}

/// 把任意 openmls 错误类型转成 [`MlsError`]。
///
/// openmls 0.9 为不同操作定义了多个错误类型（`NewGroupError`,
/// `WelcomeError`, `AddMembersError`, `MergePendingCommitError`,
/// `ExportSecretError` ...），它们不都实现 `Into<Error>`。
/// 用此 helper 统一转换。
pub fn mls_err<E: std::fmt::Debug>(e: E) -> MlsError {
    MlsError::OpenMls(format!("{:?}", e))
}
