//! 服务器 URL 解析：用户输入 → `http_url` + `ws_url` + 安全标记。
//!
//! # 输入规则
//!
//! | 用户输入 | http_url | ws_url | 安全 |
//! |---|---|---|---|
//! | `kaede.example.com` | `https://kaede.example.com` | `wss://kaede.example.com/ws` | ✅ |
//! | `kaede.example.com:8443` | `https://kaede.example.com:8443` | `wss://kaede.example.com:8443/ws` | ✅ |
//! | `https://kaede.example.com/kaede` | `https://kaede.example.com/kaede` | `wss://kaede.example.com/kaede/ws` | ✅ |
//! | `http://localhost:18080` | `http://localhost:18080` | `ws://localhost:18080/ws` | ❌（明文）|
//! | `ws://...` / `wss://...` | 拒绝 | 拒绝 | — |
//! | 空串 | 拒绝 | 拒绝 | — |
//!
//! # 为什么不允许 `ws://` 输入
//!
//! `ws_url` 由 `http_url` 推导（HTTP → WS，HTTPS → WSS），
//! 让用户只填 HTTP 层，避免混淆 + 防止 ws/wss 与 http/https 不一致。

#![forbid(unsafe_code)]

/// 解析后的服务器端点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEndpoint {
    /// HTTP 基址，如 `https://kaede.example.com`（无尾斜杠）。
    pub http_url: String,
    /// WebSocket 基址，如 `wss://kaede.example.com/ws`（无尾斜杠）。
    pub ws_url: String,
    /// 用户可见的规范化形式（去掉 scheme 和尾斜杠），用于最近列表展示。
    pub display: String,
    /// `true` = HTTPS/WSS（加密），`false` = HTTP/WS（明文）。
    pub is_secure: bool,
}

/// 解析失败的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    #[error("服务器地址不能为空")]
    Empty,
    #[error("请填 HTTP(S) 地址，不是 WS 地址（{0}）；ws/wss 会由 https/http 自动推导")]
    UseHttpNotWs(String),
    #[error("不支持的协议：{0}（只支持 http / https）")]
    UnsupportedScheme(String),
    #[error("地址格式无效：{0}")]
    Invalid(String),
}

/// 解析用户输入的服务器地址。
pub fn parse_server_url(input: &str) -> Result<ServerEndpoint, EndpointError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(EndpointError::Empty);
    }

    // 1. 剥离 scheme，推断 is_secure
    let (rest, is_secure) = if let Some(r) = s.strip_prefix("https://") {
        (r, true)
    } else if let Some(r) = s.strip_prefix("http://") {
        (r, false)
    } else if s.starts_with("ws://") || s.starts_with("wss://") {
        return Err(EndpointError::UseHttpNotWs(s.to_string()));
    } else if let Some(idx) = s.find("://") {
        return Err(EndpointError::UnsupportedScheme(s[..idx].to_string()));
    } else {
        // 无 scheme → 默认 https
        (s, true)
    };

    // 2. 分离 hostport 和 path
    let (hostport, path) = match rest.split_once('/') {
        Some((hp, p)) => (hp, p),
        None => (rest, ""),
    };
    if hostport.is_empty() {
        return Err(EndpointError::Invalid("host 为空".into()));
    }
    if hostport.contains(char::is_whitespace) {
        return Err(EndpointError::Invalid("host 含空格".into()));
    }
    // 拒绝 `user:pass@host` / `user@host`。
    // reqwest 会把 `alice@evil.com` 当作带 credentials 的 URL，可能触发
    // Authorization 头注入；UI 上 `alice@evil.com` 显示得像 alice 的主页
    // 但实际连到 evil.com——典型钓鱼面。
    if hostport.contains('@') {
        return Err(EndpointError::Invalid(
            "host 不允许包含 '@'（不支持 URL 内嵌凭据）".into(),
        ));
    }

    // 3. 规整 path：去首尾 `/`，空字符串表示无前缀
    let path = path.trim_matches('/');

    // 4. 拼接
    let http_scheme = if is_secure { "https" } else { "http" };
    let ws_scheme = if is_secure { "wss" } else { "ws" };

    let http_url = if path.is_empty() {
        format!("{http_scheme}://{hostport}")
    } else {
        format!("{http_scheme}://{hostport}/{path}")
    };

    let ws_url = if path.is_empty() {
        format!("{ws_scheme}://{hostport}/ws")
    } else {
        format!("{ws_scheme}://{hostport}/{path}/ws")
    };

    // 5. display：hostport + 可选 path（不含 scheme），便于用户识别
    let display = if path.is_empty() {
        hostport.to_string()
    } else {
        format!("{hostport}/{path}")
    };

    Ok(ServerEndpoint {
        http_url,
        ws_url,
        display,
        is_secure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_rejected() {
        assert_eq!(parse_server_url("").unwrap_err(), EndpointError::Empty);
        assert_eq!(parse_server_url("   ").unwrap_err(), EndpointError::Empty);
    }

    #[test]
    fn bare_host_defaults_to_https() {
        let e = parse_server_url("kaede.example.com").unwrap();
        assert_eq!(e.http_url, "https://kaede.example.com");
        assert_eq!(e.ws_url, "wss://kaede.example.com/ws");
        assert_eq!(e.display, "kaede.example.com");
        assert!(e.is_secure);
    }

    #[test]
    fn https_scheme_preserved() {
        let e = parse_server_url("https://kaede.example.com").unwrap();
        assert_eq!(e.http_url, "https://kaede.example.com");
        assert_eq!(e.ws_url, "wss://kaede.example.com/ws");
        assert!(e.is_secure);
    }

    #[test]
    fn http_explicit_insecure() {
        let e = parse_server_url("http://localhost:18080").unwrap();
        assert_eq!(e.http_url, "http://localhost:18080");
        assert_eq!(e.ws_url, "ws://localhost:18080/ws");
        assert_eq!(e.display, "localhost:18080");
        assert!(!e.is_secure);
    }

    #[test]
    fn port_preserved_in_both_urls() {
        let e = parse_server_url("kaede.example.com:8443").unwrap();
        assert_eq!(e.http_url, "https://kaede.example.com:8443");
        assert_eq!(e.ws_url, "wss://kaede.example.com:8443/ws");
    }

    #[test]
    fn path_prefix_preserved() {
        let e = parse_server_url("https://kaede.example.com/kaede").unwrap();
        assert_eq!(e.http_url, "https://kaede.example.com/kaede");
        assert_eq!(e.ws_url, "wss://kaede.example.com/kaede/ws");
        assert_eq!(e.display, "kaede.example.com/kaede");
    }

    #[test]
    fn trailing_slash_normalized() {
        let a = parse_server_url("https://kaede.example.com/kaede/").unwrap();
        let b = parse_server_url("https://kaede.example.com/kaede").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.http_url, "https://kaede.example.com/kaede");
        assert_eq!(a.ws_url, "wss://kaede.example.com/kaede/ws");
    }

    #[test]
    fn ws_input_rejected() {
        match parse_server_url("ws://localhost:18080") {
            Err(EndpointError::UseHttpNotWs(_)) => {}
            other => panic!("expected UseHttpNotWs, got {:?}", other),
        }
        assert!(matches!(
            parse_server_url("wss://kaede.example.com"),
            Err(EndpointError::UseHttpNotWs(_))
        ));
    }

    #[test]
    fn unknown_scheme_rejected() {
        match parse_server_url("ftp://files.example.com") {
            Err(EndpointError::UnsupportedScheme(s)) => assert_eq!(s, "ftp"),
            other => panic!("expected UnsupportedScheme, got {:?}", other),
        }
    }

    #[test]
    fn empty_host_rejected() {
        match parse_server_url("http://") {
            Err(EndpointError::Invalid(_)) => {}
            other => panic!("expected Invalid, got {:?}", other),
        }
        assert!(matches!(
            parse_server_url("http:///path"),
            Err(EndpointError::Invalid(_))
        ));
    }

    #[test]
    fn whitespace_trimmed() {
        let e = parse_server_url("  https://kaede.example.com  ").unwrap();
        assert_eq!(e.http_url, "https://kaede.example.com");
    }

    #[test]
    fn host_with_space_rejected() {
        assert!(matches!(
            parse_server_url("kaede example.com"),
            Err(EndpointError::Invalid(_))
        ));
    }

    #[test]
    fn userinfo_rejected() {
        // https://user@host / user:pass@host 全部拒绝，防钓鱼与认证头注入。
        assert!(matches!(
            parse_server_url("https://alice@evil.com"),
            Err(EndpointError::Invalid(_))
        ));
        assert!(matches!(
            parse_server_url("alice:pass@evil.com"),
            Err(EndpointError::Invalid(_))
        ));
        assert!(matches!(
            parse_server_url("http://alice@localhost:18080"),
            Err(EndpointError::Invalid(_))
        ));
    }
}
