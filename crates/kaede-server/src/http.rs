//! HTTP API：账户 + KeyPackage 目录。

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use kaede_protocol::{ServerMessage, UserId};

use crate::db::{self, base64_url};
use crate::error::ApiError;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/me", get(me))
        .route("/api/keypackages", post(publish_keypackages))
        .route("/api/keypackages/self/count", get(key_package_count))
        .route("/api/keypackages/{login_id}", get(fetch_keypackage))
        .route("/api/messages", post(post_message))
        .route("/api/messages/{channel_id}", get(get_messages))
        // B2: 消息密文上限 4 MiB（附件在 attachments router 里单独 32 MiB）
        .layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// 健康检查 / 指标
// ---------------------------------------------------------------------------

/// 就绪探针：DB 可查询才算 ready。进程活着但 DB 不可用时返回 503，
/// 让负载均衡器把流量摘走，而不是继续把请求打到坏实例上。
async fn ready(State(state): State<AppState>) -> (StatusCode, &'static str) {
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => (StatusCode::OK, "ready"),
        Err(e) => {
            tracing::warn!("readiness check failed: {}", e);
            (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
        }
    }
}

/// Prometheus text exposition。只含计数与容量，不含账号/频道/消息内容。
async fn metrics(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        crate::metrics::render(&state),
    )
}

// ---------------------------------------------------------------------------
// 注册
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RegisterReq {
    login_id: String,
    password: String,
    #[serde(default)]
    nickname: String,
    /// 设备名（可选，用于“登录设备”列表）。
    #[serde(default)]
    device_name: String,
}

#[derive(Serialize)]
struct RegisterResp {
    login_id: String,
    nickname: String,
    session_token: String,
}

async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Result<Json<RegisterResp>, ApiError> {
    validate_login_id(&req.login_id)?;
    validate_password(&req.password)?;

    // 注册是未认证入口：按来源 IP 限流，防批量注册灌水/占满连接池。
    // 反向代理部署时 peer 是代理地址，优先取代理透传的客户端 IP。
    let ip = client_ip(&headers, peer);
    if !state.check_rate_key(&format!("ip:{ip}:register"), 0.2, 10.0) {
        state.metrics.rate_limited();
        return Err(ApiError::BadRequest("registration rate limited".into()));
    }

    // P0-C：注册也要限制并发 Argon2（否则并发注册可耗尽 19GB 内存）。
    // 复用 login_semaphore（8）——register 频次远低于 login。
    let _permit = state
        .login_semaphore
        .acquire()
        .await
        .map_err(|_| ApiError::Internal("register queue closed".into()))?;

    // C-9: 移除 SELECT-then-INSERT TOCTOU，改为 INSERT ... ON CONFLICT DO NOTHING
    // + 事务包裹 accounts / members 双写，避免半提交
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(req.password.as_bytes(), &salt)?
        .to_string();

    // H16: nickname 长度 + 控制字符校验（对齐 members::update_member）
    if req.nickname.chars().count() > 32 {
        return Err(ApiError::BadRequest(
            "nickname too long (max 32 chars)".into(),
        ));
    }
    if req.nickname.chars().any(|c| c.is_control()) {
        return Err(ApiError::BadRequest(
            "nickname contains control chars".into(),
        ));
    }
    let nickname = if req.nickname.trim().is_empty() {
        req.login_id.clone()
    } else {
        req.nickname.clone()
    };

    let now = db::now_ms();

    // 被管理员删除的 login_id 永久退役，不能被自动注册/重新注册重建。
    // 返回 409 与“已占用”一致，不额外暴露删除历史。
    //
    // 在事务外读取：若先 SELECT 再在事务内 INSERT，SQLite 会把读锁
    // 升级为写锁，并发注册时容易出现 SQLITE_BUSY。退役表与 accounts
    // 的竞态窗口极窄，且不会让已退役账号获得任何特权。
    let retired: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM retired_logins WHERE login_id = ?")
        .bind(&req.login_id)
        .fetch_optional(&state.pool)
        .await?;
    if retired.is_some() {
        return Err(ApiError::Conflict("login_id_taken".into()));
    }

    let mut tx = state.pool.begin().await?;

    let res = sqlx::query(
        "INSERT INTO accounts (login_id, password_hash, nickname, created_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT(login_id) DO NOTHING",
    )
    .bind(&req.login_id)
    .bind(&hash)
    .bind(&nickname)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    if res.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(ApiError::Conflict("login_id_taken".into()));
    }

    // 同步写 members 表。P1-9：用 SQL 子查询原子判 op5，消除并发注册竞态。
    sqlx::query(
        "INSERT INTO members (login_id, nickname, op, bio, title, avatar_url, joined_at, updated_at) \
         VALUES (?, ?, (SELECT CASE WHEN COUNT(*) = 0 THEN 'op5' ELSE 'op2' END FROM members), '', '', '', ?, ?)",
    )
    .bind(&req.login_id)
    .bind(&nickname)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let token = issue_session(&state, &req.login_id, &req.device_name).await?;

    Ok(Json(RegisterResp {
        login_id: req.login_id,
        nickname,
        session_token: token,
    }))
}

// ---------------------------------------------------------------------------
// 登录
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginReq {
    login_id: String,
    password: String,
    /// 设备名（可选，用于“登录设备”列表）。
    #[serde(default)]
    device_name: String,
}

#[derive(Serialize)]
struct LoginResp {
    login_id: String,
    nickname: String,
    session_token: String,
}

/// 不存在的账号也要跑一次 Argon2，避免通过响应时间枚举 login_id。
fn dummy_password_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(b"kaede-dummy-password", &salt)
            .map(|h| h.to_string())
            .unwrap_or_default()
    })
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Result<Json<LoginResp>, ApiError> {
    // P0-2：并发 Argon2 限制（每请求 19MiB，8 并发 = 152MiB 上限）。
    let _permit = state
        .login_semaphore
        .acquire()
        .await
        .map_err(|_| ApiError::Internal("login queue closed".into()))?;

    // P0-A 修复：失败计数检查必须在 semaphore 之后——否则并发 100 个请求
    // 全部通过检查才排队，semaphore 出来后不再复查 → 5 次锁完全失效。
    // 失败计数按 (login_id, 来源 IP) 隔离：否则任意人连发 5 次错误密码
    // 就能把已知账号锁死 15 分钟。
    let ip = client_ip(&headers, peer);
    let failure_key = (req.login_id.clone(), ip);
    {
        let mut failures = state
            .login_failures
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((count, first)) = failures.get(&failure_key) {
            if *count >= 5 && first.elapsed() < std::time::Duration::from_secs(900) {
                return Err(ApiError::Unauthorized);
            }
            if first.elapsed() >= std::time::Duration::from_secs(900) {
                failures.remove(&failure_key);
            }
        }
    }

    let row: Option<(String, String, i64)> =
        sqlx::query_as("SELECT password_hash, nickname, banned FROM accounts WHERE login_id = ?")
            .bind(&req.login_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((hash, nickname, banned)) = row else {
        // 账号不存在：跑一次等价的 Argon2 验证，抹平与“密码错误”的时序差异。
        if let Ok(parsed) = PasswordHash::new(dummy_password_hash()) {
            let _ = Argon2::default().verify_password(req.password.as_bytes(), &parsed);
        }
        return Err(ApiError::Unauthorized);
    };

    // 封禁账号：与不存在账号同样返回 401，避免枚举
    if banned != 0 {
        return Err(ApiError::Unauthorized);
    }

    let parsed =
        PasswordHash::new(&hash).map_err(|_| ApiError::Internal("bad stored hash".into()))?;
    if Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed)
        .is_err()
    {
        let mut failures = state
            .login_failures
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let entry = failures
            .entry(failure_key)
            .or_insert((0, std::time::Instant::now()));
        entry.0 += 1;
        return Err(ApiError::Unauthorized);
    }

    state
        .login_failures
        .lock()
        .unwrap()
        .remove(&(req.login_id.clone(), peer.ip()));

    let token = issue_session(&state, &req.login_id, &req.device_name).await?;

    Ok(Json(LoginResp {
        login_id: req.login_id,
        nickname,
        session_token: token,
    }))
}

// ---------------------------------------------------------------------------
// /me
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MeResp {
    login_id: String,
    nickname: String,
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<MeResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let row: Option<(String,)> = sqlx::query_as("SELECT nickname FROM accounts WHERE login_id = ?")
        .bind(uid.as_str())
        .fetch_optional(&state.pool)
        .await?;
    let (nickname,) = row.ok_or(ApiError::NotFound("account".into()))?;
    Ok(Json(MeResp {
        login_id: uid.0,
        nickname,
    }))
}

// ---------------------------------------------------------------------------
// KeyPackage 目录
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PublishKpReq {
    /// base64(URL-safe, no pad) 的 KeyPackage 列表。
    packages: Vec<String>,
}

async fn publish_keypackages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PublishKpReq>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    if req.packages.is_empty() || req.packages.len() > 100 {
        return Err(ApiError::BadRequest("packages count 1..=100".into()));
    }

    let now = db::now_ms();
    for pkg in &req.packages {
        let bytes = base64_decode(pkg)?;
        if bytes.len() > 8192 {
            return Err(ApiError::BadRequest("keypackage too large".into()));
        }
        // C8: 校验 KP 内部 credential 与 auth uid 一致。
        // 没有这一步，attacker 可上传 credential="victim" 的 KP，
        // 任何人 fetch 后被 server 返回，加入群时冒充 victim（MLS 层无防护）。
        let cred = kaede_mls::extract_credential_from_kp(&bytes)
            .map_err(|e| ApiError::BadRequest(format!("invalid keypackage: {}", e)))?;
        if cred != uid.as_str() {
            return Err(ApiError::BadRequest(format!(
                "keypackage credential mismatch: expected {}, got {}",
                uid.as_str(),
                cred
            )));
        }
        sqlx::query("INSERT INTO key_packages (login_id, package, created_at) VALUES (?, ?, ?)")
            .bind(uid.as_str())
            .bind(bytes)
            .bind(now)
            .execute(&state.pool)
            .await?;
    }

    // 每个用户只保留最新 100 个 KP，避免长期登录导致队列无限增长。
    sqlx::query(
        "DELETE FROM key_packages WHERE login_id = ? AND id NOT IN \
         (SELECT id FROM key_packages WHERE login_id = ? ORDER BY id DESC LIMIT 100)",
    )
    .bind(uid.as_str())
    .bind(uid.as_str())
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct FetchKpResp {
    target: String,
    /// base64(URL-safe, no pad)
    package: String,
}

#[derive(Serialize)]
struct KpCountResp {
    remaining: i64,
}

/// 非好友请求者能消费到的最低剩余量。
///
/// 队列低于该值时只允许好友继续取，攻击者无法把受害者的队列抽干到
/// “任何人都无法邀请”的地步；受害者在下次登录时由客户端自动补货。
const KP_RESERVE: i64 = 8;

/// 当前账号自己的 KeyPackage 队列剩余量。
///
/// 客户端登录时据此决定是否补货：每个 KP 只能消费一次，长期使用后队列会自然耗尽。
async fn key_package_count(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<KpCountResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;
    let (remaining,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM key_packages WHERE login_id = ?")
            .bind(uid.as_str())
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(KpCountResp { remaining }))
}

async fn fetch_keypackage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(login_id): Path<String>,
) -> Result<Json<FetchKpResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    // 防抽干第一层：单个请求者全局限流（不能同时扫很多人的队列）。
    if !state.check_rate_scoped(&uid, "kp_fetch", 1.0, 10.0) {
        state.metrics.rate_limited();
        return Err(ApiError::TooManyRequests(
            "keypackage fetch rate limited".into(),
        ));
    }

    // 防抽干第一层（补充）：单个请求者对同一 target 的消费速率上限
    // （约 1 次 / 10s，突发 2）。即使攻击者换账号，单账号也无法快速清空队列。
    let pair_key = format!("kp_pair:{}:{}", uid.as_str(), login_id);
    if !state.check_rate_key(&pair_key, 0.1, 2.0) {
        state.metrics.rate_limited();
        return Err(ApiError::TooManyRequests(
            "keypackage fetch rate limited".into(),
        ));
    }

    // 好友（双向 accepted）不受目标预算与保留量限制。
    // 好友关系必须对方接受才能建立，攻击者无法单方面获取该豁免。
    let is_friend = crate::friends::is_friend(&state.pool, uid.as_str(), &login_id)
        .await
        .map_err(|e| ApiError::Internal(format!("db: {}", e)))?;

    if !is_friend {
        // 防抽干第二层：per-target 全局限流（防多账号轮换）。
        if !state.check_kp_fetch(&login_id) {
            state.metrics.rate_limited();
            tracing::warn!(
                "keypackage fetch throttled: target={} requester={}",
                login_id,
                uid
            );
            return Err(ApiError::TooManyRequests(
                "keypackage fetch throttled".into(),
            ));
        }

        // 防抽干第三层：保留量。队列被消耗到只剩少量时，只有好友还能继续取。
        // 这样攻击者最多把队列压到保留线，无法彻底抽干到“无法被任何人邀请”。
        let (remaining,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM key_packages WHERE login_id = ?")
                .bind(&login_id)
                .fetch_one(&state.pool)
                .await?;
        if remaining == 0 {
            return Err(ApiError::NotFound("key_package".into()));
        }
        if remaining <= KP_RESERVE {
            state.metrics.rate_limited();
            tracing::warn!(
                "keypackage queue reserved: target={} requester={} remaining={}",
                login_id,
                uid,
                remaining
            );
            return Err(ApiError::TooManyRequests(
                "keypackage queue reserved".into(),
            ));
        }
    }

    // P1-4：DELETE ... RETURNING 原子取走 + 删除，无 TOCTOU（SQLite >= 3.35）。
    let row: Option<(Vec<u8>,)> = sqlx::query_as(
        "DELETE FROM key_packages \
         WHERE id = (SELECT id FROM key_packages WHERE login_id = ? ORDER BY id LIMIT 1) \
         RETURNING package",
    )
    .bind(&login_id)
    .fetch_optional(&state.pool)
    .await?;

    let (bytes,) = row.ok_or(ApiError::NotFound("key_package".into()))?;

    Ok(Json(FetchKpResp {
        target: login_id,
        package: base64_url(&bytes),
    }))
}

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

async fn issue_session(
    state: &AppState,
    login_id: &str,
    device_name: &str,
) -> Result<String, ApiError> {
    let token = db::random_token();
    let session_id = db::random_token();
    let now = db::now_ms();
    let expires_at = now + state.cfg.session_ttl_hours * 3600 * 1000;
    let device_name = truncate_chars(device_name.trim(), 64);
    // 只存 token 的 SHA-256。DB 泄漏时无法直接拿去登录。
    sqlx::query(
        "INSERT INTO sessions \
         (token, login_id, expires_at, device_name, created_at, last_seen_at, session_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(db::hash_token(&token))
    .bind(login_id)
    .bind(expires_at)
    .bind(&device_name)
    .bind(now)
    .bind(now)
    .bind(&session_id)
    .execute(&state.pool)
    .await?;
    Ok(token)
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub(crate) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<UserId, ApiError> {
    Ok(authenticate_session(state, headers).await?.0)
}

/// 认证并返回当前请求的 `(uid, session_id)`。
pub(crate) async fn authenticate_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(UserId, String), ApiError> {
    let token = match headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        Some(t) => t,
        None => {
            state.metrics.auth_failure();
            return Err(ApiError::Unauthorized);
        }
    };

    // 只接受 hash(token) 匹配。不能再兼容明文 token：否则 DB 泄漏时
    // 库里保存的 hash 本身就能直接当 Bearer 使用。
    let row: Option<(String, i64, String)> =
        sqlx::query_as("SELECT login_id, expires_at, session_id FROM sessions WHERE token = ?")
            .bind(db::hash_token(token))
            .fetch_optional(&state.pool)
            .await?;
    let (login_id, expires_at, session_id) = match row {
        Some(r) => r,
        None => {
            state.metrics.auth_failure();
            return Err(ApiError::Unauthorized);
        }
    };
    if expires_at < db::now_ms() {
        state.metrics.auth_failure();
        return Err(ApiError::Unauthorized);
    }
    Ok((UserId::new(login_id), session_id))
}

/// 取客户端 IP。
///
/// 反向代理部署时 `ConnectInfo` 是代理地址；只有在 peer 本身是回环/私网
/// （即预期的本机或容器内代理）时才信任 `X-Forwarded-For`，避免公网直连
/// 时伪造 header 绕过注册限流/登录失败计数。
fn client_ip(headers: &HeaderMap, peer: std::net::SocketAddr) -> std::net::IpAddr {
    let peer_ip = peer.ip();
    let trusted_proxy = match peer_ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    };
    if !trusted_proxy {
        return peer_ip;
    }
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse().ok());
    let real_ip = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok());
    forwarded.or(real_ip).unwrap_or(peer_ip)
}

fn validate_login_id(s: &str) -> Result<(), ApiError> {
    if s.len() < 3 || s.len() > 39 {
        return Err(ApiError::BadRequest("login_id length 3..=39".into()));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ApiError::BadRequest("login_id charset".into()));
    }
    if s.starts_with('-') || s.ends_with('-') || s.contains("--") {
        return Err(ApiError::BadRequest("login_id hyphens".into()));
    }
    Ok(())
}

fn validate_password(s: &str) -> Result<(), ApiError> {
    if s.len() < 8 || s.len() > 128 {
        return Err(ApiError::BadRequest("password length 8..=128".into()));
    }
    let has_alpha = s.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    if !has_alpha || !has_digit {
        return Err(ApiError::BadRequest("password needs letter + digit".into()));
    }
    Ok(())
}

fn base64_decode(s: &str) -> Result<Vec<u8>, ApiError> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| ApiError::BadRequest(format!("base64: {}", e)))
}

// ---------------------------------------------------------------------------
// 消息持久化
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PostMessageReq {
    channel_id: String,
    /// P1-5：客户端自报的 ts 不再使用，服务器用 now_ms() 覆盖。
    /// 保留字段仅为反序列化兼容，旧客户端继续带此字段。
    #[allow(dead_code)]
    ts: i64,
    /// 客户端自报的 sender。
    ///
    /// **服务器故意不读此字段**——用 auth uid 覆盖，防止冒充。
    /// 保留字段是为了兼容客户端旧版本的 CBOR/JSON 反序列化。
    #[allow(dead_code)]
    sender: String,
    /// base64(URL-safe, no pad) 的 MLS Application Message 密文。
    ciphertext: String,
}

#[derive(Serialize)]
struct MessageRecord {
    ts: i64,
    sender: String,
    ciphertext: String,
}

#[derive(Serialize)]
struct MessagesResp {
    messages: Vec<MessageRecord>,
}

async fn post_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<StatusCode, ApiError> {
    // C1：用 auth uid 覆盖客户端自报的 sender，防止冒充
    let uid = authenticate(&state, &headers).await?;

    // C11: per-uid 限流，防 DB flood
    if !state.check_rate_scoped(&uid, "messages", 10.0, 30.0) {
        state.metrics.rate_limited();
        return Err(ApiError::BadRequest("message rate limited".into()));
    }

    if req.channel_id.is_empty() || req.ciphertext.is_empty() {
        return Err(ApiError::BadRequest(
            "channel_id / ciphertext required".into(),
        ));
    }
    let ct = base64_decode(&req.ciphertext)?;
    if ct.len() > 4 * 1024 * 1024 {
        return Err(ApiError::BadRequest("message too large".into()));
    }
    // 字节预算：条数限流之外单独限制持续写入带宽（1 MiB/s，突发 8 MiB）。
    if !state.check_rate_scoped_cost(
        &uid,
        "messages-bytes",
        1024.0 * 1024.0,
        8.0 * 1024.0 * 1024.0,
        ct.len() as f64,
    ) {
        state.metrics.rate_limited();
        return Err(ApiError::BadRequest("message byte budget exceeded".into()));
    }

    // C2：channel_id 必须存在（DM 走 dm.rs，不进这里）。
    // 私密频道：仅 channel_members 成员可发送。
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT id, is_private FROM channels WHERE id = ?")
            .bind(&req.channel_id)
            .fetch_optional(&state.pool)
            .await?;
    let (_, is_private_raw) =
        row.ok_or_else(|| ApiError::NotFound(format!("channel: {}", req.channel_id)))?;
    if is_private_raw != 0 {
        let member: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?")
                .bind(&req.channel_id)
                .bind(uid.as_str())
                .fetch_optional(&state.pool)
                .await?;
        if member.is_none() {
            return Err(ApiError::Unauthorized);
        }
        // 私密频道只接受频道 group 的密文（首字节 0x03）。
        // 若客户端本地群丢失后退回主群 MLS 加密，会让全服主群成员都能解密，
        // 私密性被静默破坏 —— 必须拒绝。
        if ct.first() != Some(&0x03) {
            return Err(ApiError::BadRequest(
                "private channel messages must use the channel group key (0x03 prefix)".into(),
            ));
        }
    }

    // P1-5：忽略客户端 ts，用服务器时间，防时间线污染。
    //
    // 同一频道内 ts 严格递增（原子 SQL 计算），否则同毫秒消息会在
    // `ts < before_ts` 分页时被跳过。
    let now = db::now_ms();
    let mut tx = state.pool.begin().await?;

    // 配额预占和消息 INSERT 必须在同一事务：超额拒绝，INSERT 失败时
    // 配额自动回滚，不会出现“扣了额度但消息没落库”。
    if !db::reserve_message_bytes(&mut tx, uid.as_str(), ct.len() as i64).await? {
        tx.rollback().await?;
        return Err(ApiError::BadRequest(format!(
            "message quota exceeded (max {} MiB)",
            db::MAX_USER_MESSAGE_BYTES / 1024 / 1024
        )));
    }

    let (ts,): (i64,) = sqlx::query_as(
        "INSERT INTO messages (channel_id, ts, sender, ciphertext) \
         SELECT ?, CASE WHEN ? > COALESCE(MAX(ts), 0) THEN ? ELSE COALESCE(MAX(ts), 0) + 1 END, ?, ? \
         FROM messages WHERE channel_id = ? \
         RETURNING ts",
    )
    .bind(&req.channel_id)
    .bind(now)
    .bind(now)
    .bind(uid.as_str()) // ← 服务器权威
    .bind(&ct)
    .bind(&req.channel_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    state.metrics.message_stored();

    // 私密频道：只把密文实时推给频道成员（公开频道由客户端 MLS 广播）。
    // 这里使用服务器权威 ts，客户端落 UI 时据此排序/去重。
    if is_private_raw != 0 {
        let members: Vec<(String,)> = sqlx::query_as(
            "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ?",
        )
        .bind(&req.channel_id)
        .bind(uid.as_str())
        .fetch_all(&state.pool)
        .await?;
        for (member,) in members {
            state.send_to(
                &UserId::new(member),
                ServerMessage::ChannelMessage {
                    channel_id: req.channel_id.clone(),
                    from: uid.clone(),
                    ts,
                    ciphertext: ct.clone(),
                },
            );
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn get_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<GetMessagesQuery>,
) -> Result<Json<MessagesResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    // 频道必须存在（防信息泄露：区分"频道不存在"和"无权限"）
    let row: Option<(i64,)> = sqlx::query_as("SELECT is_private FROM channels WHERE id = ?")
        .bind(&channel_id)
        .fetch_optional(&state.pool)
        .await?;
    let (is_private_raw,) =
        row.ok_or_else(|| ApiError::NotFound(format!("channel: {}", channel_id)))?;
    // 私密频道：仅成员可读
    if is_private_raw != 0 {
        let member: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM channel_members WHERE channel_id = ? AND login_id = ?")
                .bind(&channel_id)
                .bind(uid.as_str())
                .fetch_optional(&state.pool)
                .await?;
        if member.is_none() {
            return Err(ApiError::Unauthorized);
        }
    }

    let before = q.before_ts.unwrap_or(i64::MAX);
    let limit = q.limit.unwrap_or(200).clamp(1, 500); // C-8: clamp 防 limit=-1 全表扫描

    // 私密频道隐藏“历史遗留”的主群密文（非 0x03 前缀）：
    // 那是旧版本 bug 造成的，所有主群成员都能解；虽然服务端无法重新加密，
    // 至少不再继续下发。
    let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT ts, sender, ciphertext FROM messages \
         WHERE channel_id = ? AND ts < ? \
           AND (? = 0 OR substr(ciphertext, 1, 1) = X'03') \
         ORDER BY ts DESC LIMIT ?",
    )
    .bind(&channel_id)
    .bind(before)
    .bind(is_private_raw)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    // 反序成时间升序
    let mut out: Vec<MessageRecord> = rows
        .into_iter()
        .map(|(ts, sender, ct)| MessageRecord {
            ts,
            sender,
            ciphertext: base64_url(&ct),
        })
        .collect();
    out.reverse();

    Ok(Json(MessagesResp { messages: out }))
}

#[derive(Deserialize)]
struct GetMessagesQuery {
    before_ts: Option<i64>,
    limit: Option<i64>,
}
