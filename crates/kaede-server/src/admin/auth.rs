//! Admin 认证：从 X-Admin-Token header 校验 token。
//!
//! 用 SHA-256 把两个 token 归一化成定长 32 字节，再用 constant-time 比较，
//! 避免通过响应时间逐字节猜 token。单独的 admin token，和用户 session 隔离。

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::ApiError;
use crate::state::AppState;

pub struct AdminAuth;

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // 空 token 必须视为“未启用”，否则空 header 会与空期望值匹配（fail-open）。
        if state.admin_token.is_empty() {
            return Err(ApiError::Unauthorized);
        }
        let token = parts
            .headers
            .get("x-admin-token")
            .and_then(|v| v.to_str().ok())
            .filter(|t| !t.is_empty())
            .ok_or(ApiError::Unauthorized)?;
        let provided: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let expected: [u8; 32] = Sha256::digest(state.admin_token.as_bytes()).into();
        if provided.ct_eq(&expected).unwrap_u8() != 1 {
            return Err(ApiError::Unauthorized);
        }
        Ok(AdminAuth)
    }
}
