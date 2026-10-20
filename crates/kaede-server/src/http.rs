//! HTTP API：账户 + KeyPackage 目录。

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use kaede_protocol::UserId;

use crate::db::{self, base64_url};
use crate::error::ApiError;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/me", get(me))
        .route("/api/keypackages", post(publish_keypackages))
        .route("/api/keypackages/{login_id}", get(fetch_keypackage))
        .route("/api/messages", post(post_message))
        .route("/api/messages/{channel_id}", get(get_messages))
        .with_state(state)
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
}

#[derive(Serialize)]
struct RegisterResp {
    login_id: String,
    nickname: String,
    session_token: String,
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterReq>,
) -> Result<Json<RegisterResp>, ApiError> {
    validate_login_id(&req.login_id)?;
    validate_password(&req.password)?;

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

    let nickname = if req.nickname.trim().is_empty() {
        req.login_id.clone()
    } else {
        req.nickname.clone()
    };

    let now = db::now_ms();

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

    let token = issue_session(&state, &req.login_id).await?;

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
}

#[derive(Serialize)]
struct LoginResp {
    login_id: String,
    nickname: String,
    session_token: String,
}

async fn login(
    State(state): State<AppState>,
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
    {
        let mut failures = state.login_failures.lock().unwrap();
        if let Some((count, first)) = failures.get(&req.login_id) {
            if *count >= 5 && first.elapsed() < std::time::Duration::from_secs(900) {
                return Err(ApiError::Unauthorized);
            }
            if first.elapsed() >= std::time::Duration::from_secs(900) {
                failures.remove(&req.login_id);
            }
        }
    }

    let row: Option<(String, String)> =
        sqlx::query_as("SELECT password_hash, nickname FROM accounts WHERE login_id = ?")
            .bind(&req.login_id)
            .fetch_optional(&state.pool)
            .await?;
    let (hash, nickname) = row.ok_or(ApiError::Unauthorized)?;

    let parsed =
        PasswordHash::new(&hash).map_err(|_| ApiError::Internal("bad stored hash".into()))?;
    if Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed)
        .is_err()
    {
        let mut failures = state.login_failures.lock().unwrap();
        let entry = failures
            .entry(req.login_id.clone())
            .or_insert((0, std::time::Instant::now()));
        entry.0 += 1;
        return Err(ApiError::Unauthorized);
    }

    state.login_failures.lock().unwrap().remove(&req.login_id);

    let token = issue_session(&state, &req.login_id).await?;

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
        sqlx::query("INSERT INTO key_packages (login_id, package, created_at) VALUES (?, ?, ?)")
            .bind(uid.as_str())
            .bind(bytes)
            .bind(now)
            .execute(&state.pool)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct FetchKpResp {
    target: String,
    /// base64(URL-safe, no pad)
    package: String,
}

async fn fetch_keypackage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(login_id): Path<String>,
) -> Result<Json<FetchKpResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    // C-7: per-uid 限流，防恶意用户消耗他人 KeyPackage 队列
    if !state.check_rate(&uid, 1.0, 10.0) {
        return Err(ApiError::BadRequest("keypackage fetch rate limited".into()));
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

async fn issue_session(state: &AppState, login_id: &str) -> Result<String, ApiError> {
    let token = db::random_token();
    let expires_at = db::now_ms() + state.cfg.session_ttl_hours * 3600 * 1000;
    sqlx::query("INSERT INTO sessions (token, login_id, expires_at) VALUES (?, ?, ?)")
        .bind(&token)
        .bind(login_id)
        .bind(expires_at)
        .execute(&state.pool)
        .await?;
    Ok(token)
}

pub(crate) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<UserId, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;

    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT login_id, expires_at FROM sessions WHERE token = ?")
            .bind(token)
            .fetch_optional(&state.pool)
            .await?;
    let (login_id, expires_at) = row.ok_or(ApiError::Unauthorized)?;
    if expires_at < db::now_ms() {
        return Err(ApiError::Unauthorized);
    }
    Ok(UserId::new(login_id))
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

    if req.channel_id.is_empty() || req.ciphertext.is_empty() {
        return Err(ApiError::BadRequest(
            "channel_id / ciphertext required".into(),
        ));
    }
    let ct = base64_decode(&req.ciphertext)?;
    if ct.len() > 4 * 1024 * 1024 {
        return Err(ApiError::BadRequest("message too large".into()));
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
    }

    // P1-5：忽略客户端 ts，用服务器时间，防时间线污染。
    let now = db::now_ms();
    sqlx::query("INSERT INTO messages (channel_id, ts, sender, ciphertext) VALUES (?, ?, ?, ?)")
        .bind(&req.channel_id)
        .bind(now)
        .bind(uid.as_str()) // ← 服务器权威
        .bind(ct)
        .execute(&state.pool)
        .await?;

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

    let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT ts, sender, ciphertext FROM messages \
         WHERE channel_id = ? AND ts < ? \
         ORDER BY ts DESC LIMIT ?",
    )
    .bind(&channel_id)
    .bind(before)
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
