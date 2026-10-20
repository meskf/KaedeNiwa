//! 客户端生命周期状态机。
//!
//! 状态必须按 [`ClientState::can_transition_to`] 里定义的路径流转。
//! 任何非法跳转都返回 [`TransitionError`]。

/// 客户端生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientState {
    /// 初始态。未连接，未认证。
    Idle,
    /// 正在与 signaling server 建立连接。
    Connecting,
    /// 已认证，持有 session token，但未加入 MLS 群组。
    Authenticated,
    /// 正在加入 / 创建 MLS 群组。
    MlsJoining,
    /// MLS 群组已就绪，可以收发 Application Message。
    MlsReady,
    /// 连接断开。
    Disconnected,
}

impl ClientState {
    /// 允许的状态转移。
    pub fn can_transition_to(self, next: Self) -> bool {
        use ClientState::*;
        matches!(
            (self, next),
            (Idle, Connecting)
                | (Connecting, Authenticated)
                | (Connecting, Disconnected)
                | (Connecting, Idle)
                | (Authenticated, MlsJoining)
                | (Authenticated, Disconnected)
                | (MlsJoining, MlsReady)
                | (MlsJoining, Authenticated)
                | (MlsJoining, Disconnected)
                | (MlsReady, MlsJoining)
                | (MlsReady, Disconnected)
                | (Disconnected, Connecting)
                | (Disconnected, Idle)
        )
    }

    /// P1-17: Idle 是初态（可转 Connecting），不是终态。保留旧名兼容。
    pub fn is_terminal(self) -> bool {
        self.is_initial()
    }

    pub fn is_initial(self) -> bool {
        matches!(self, ClientState::Idle)
    }

    pub fn is_ready(self) -> bool {
        matches!(self, ClientState::MlsReady)
    }
}

/// 状态转移错误。
#[derive(Debug, thiserror::Error)]
pub enum TransitionError {
    #[error("invalid state transition: {from:?} -> {to:?}")]
    Invalid { from: ClientState, to: ClientState },
}

/// 客户端状态容器。可被 UI 订阅。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientStateMachine {
    state: ClientState,
}

impl ClientStateMachine {
    pub fn new() -> Self {
        Self {
            state: ClientState::Idle,
        }
    }

    pub fn state(&self) -> ClientState {
        self.state
    }

    pub fn try_transition(&mut self, next: ClientState) -> Result<(), TransitionError> {
        if !self.state.can_transition_to(next) {
            return Err(TransitionError::Invalid {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }

    /// 强制执行转移（仅测试 / 恢复场景使用）。
    pub fn force(&mut self, next: ClientState) {
        self.state = next;
    }
}

impl Default for ClientStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path() {
        let mut m = ClientStateMachine::new();
        assert!(m.try_transition(ClientState::Connecting).is_ok());
        assert!(m.try_transition(ClientState::Authenticated).is_ok());
        assert!(m.try_transition(ClientState::MlsJoining).is_ok());
        assert!(m.try_transition(ClientState::MlsReady).is_ok());
        assert!(m.state().is_ready());
    }

    #[test]
    fn invalid_jump_rejected() {
        let mut m = ClientStateMachine::new();
        assert!(m.try_transition(ClientState::MlsReady).is_err());
        assert!(m.try_transition(ClientState::Authenticated).is_err());
    }

    #[test]
    fn disconnect_recovery() {
        let mut m = ClientStateMachine::new();
        m.try_transition(ClientState::Connecting).unwrap();
        m.try_transition(ClientState::Authenticated).unwrap();
        m.try_transition(ClientState::MlsJoining).unwrap();
        m.try_transition(ClientState::MlsReady).unwrap();
        m.try_transition(ClientState::Disconnected).unwrap();
        assert!(m.try_transition(ClientState::Connecting).is_ok());
    }

    #[test]
    fn mls_rejoin_on_member_change() {
        let mut m = ClientStateMachine::new();
        m.force(ClientState::MlsReady);
        assert!(m.try_transition(ClientState::MlsJoining).is_ok());
        assert!(m.try_transition(ClientState::MlsReady).is_ok());
    }

    #[test]
    fn join_failure_falls_back() {
        let mut m = ClientStateMachine::new();
        m.force(ClientState::Authenticated);
        m.try_transition(ClientState::MlsJoining).unwrap();
        m.try_transition(ClientState::Authenticated).unwrap();
    }
}
