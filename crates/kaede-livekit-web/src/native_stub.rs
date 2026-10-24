//! 非 wasm32 平台：统一 stub，返回 NotSupported。

use crate::error::LiveKitWebError;

pub struct LiveKitWebSession;

impl LiveKitWebSession {
    pub async fn new(_e2ee_key: Option<[u8; 32]>) -> Result<Self, LiveKitWebError> {
        Err(LiveKitWebError::NotSupported)
    }
    pub async fn connect(&self, _url: &str, _token: &str) -> Result<(), LiveKitWebError> {
        Err(LiveKitWebError::NotSupported)
    }
    pub async fn set_screen_share(
        &self,
        _enabled: bool,
        _profile: kaede_protocol::VideoProfile,
    ) -> Result<(), LiveKitWebError> {
        Err(LiveKitWebError::NotSupported)
    }
    pub async fn set_microphone(&self, _enabled: bool) -> Result<(), LiveKitWebError> {
        Err(LiveKitWebError::NotSupported)
    }
    pub async fn set_speaker_enabled(&self, _enabled: bool) -> Result<(), LiveKitWebError> {
        Err(LiveKitWebError::NotSupported)
    }
    pub fn attach_remote_videos_to(&self, _container_id: &str) -> Result<(), LiveKitWebError> {
        Err(LiveKitWebError::NotSupported)
    }
    pub async fn disconnect(&self) -> Result<(), LiveKitWebError> {
        Err(LiveKitWebError::NotSupported)
    }
}
