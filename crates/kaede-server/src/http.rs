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

#[derive(serde::Serialize)]
#[allow(dead_code)]
struct JoinChannelResp {
    is_first: bool,
}

// ---------------------------------------------------------------------------
// 健康检查 / 指标
// ---------------------------------------------------------------------------

/// 就绪探针：DB 可查询才算 ready。进程活着但 DB 不可用时返回 503，
/// 让负载均衡器把流量摘走，而不是继续把请求打到坏实例上。
async fn ready(State(state): State<AppState>) -> (StatusCode, &'static str) {
    if let Err(e) = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        tracing::warn!("readiness check failed: {}", e);
        return (StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
    }
    if let Err(e) = crate::attachments::storage_ready() {
        tracing::warn!("readiness: {}", e);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "attachment storage unavailable",
        );
    }
    (StatusCode::OK, "ready")
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

/// 获取一次 Argon2 并发许可。
///
/// 队列满时最多等 10 秒，然后返回 429，而不是让请求无限期挂起——
/// 否则并发登录/建频道可以把 semaphore 队列撑爆。
pub(crate) async fn acquire_argon2(
    state: &AppState,
) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
    acquire_argon2_weighted(state, 1).await
}

/// 获取 `permits` 份 Argon2 并发许可。
///
/// Argon2 的内存占用与档位成正比（19 / 64 / 256 MiB）。并发上限按最小档
/// （8 × 19 MiB）估算；256 MiB 档必须按 4 份额度计费，否则 8 个并发请求
/// 会瞬时占用 2 GiB，足以打爆小内存 VPS。
pub(crate) async fn acquire_argon2_weighted(
    state: &AppState,
    permits: u32,
) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
    let permits = permits.clamp(1, 8);
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state.login_semaphore.clone().acquire_many_owned(permits),
    )
    .await
    .map_err(|_| ApiError::TooManyRequests("crypto work queue saturated".into()))?
    .map_err(|_| ApiError::Internal("crypto work queue closed".into()))
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
    let _permit = acquire_argon2(&state).await?;

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
    //
    // dev-mode：非首个用户默认 op3（普管），方便本地测试创建/删除频道。
    // 生产构建（无 dev-mode feature）：非首个用户默认 op2（成员）。
    let op_subquery = if cfg!(feature = "dev-mode") {
        "(SELECT CASE WHEN COUNT(*) = 0 THEN 'op5' ELSE 'op3' END FROM members)"
    } else {
        "(SELECT CASE WHEN COUNT(*) = 0 THEN 'op5' ELSE 'op2' END FROM members)"
    };
    // sqlx 0.9: op_subquery 来自 cfg! 分支的 &'static str，不涉及用户输入
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO members (login_id, nickname, op, bio, title, avatar_url, joined_at, updated_at) \
         VALUES (?, ?, {}, '', '', '', ?, ?)",
        op_subquery
    )))
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
    let _permit = acquire_argon2(&state).await?;

    // P0-A 修复：失败计数检查必须在 semaphore 之后——否则并发 100 个请求
    // 全部通过检查才排队，semaphore 出来后不再复查 → 5 次锁完全失效。
    // 失败计数按 (login_id, 来源 IP) 隔离：否则任意人连发 5 次错误密码
    // 就能把已知账号锁死 15 分钟。
    let ip = client_ip(&headers, peer);

    // 同一来源 IP 的登录总速率上限：防止用海量不存在的 login_id 触发
    // Argon2（每个 19 MiB / 20ms）把 CPU 打满。
    //
    // 阈值刻意放宽到 20/s burst 200：NAT/公司出口后可能有很多正常用户，
    // 且并发 Argon2 已由 semaphore(8) 限制，内存上限仍有界。
    if !state.check_rate_key(&format!("ip:{ip}:login"), 20.0, 200.0) {
        state.metrics.rate_limited();
        return Err(ApiError::TooManyRequests(
            "login rate limited from this address".into(),
        ));
    }
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

    // BUG-FIX: 失败计数按 `client_ip()` 记录，清除必须用同一个来源 IP。
    // 之前用 `peer.ip()`，反向代理部署下（peer 是代理地址）计数永远清不掉。
    state
        .login_failures
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(req.login_id.clone(), ip));

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
    /// 身份重建后首次上传时置 true：先清空该账号的旧 KP 队列。
    #[serde(default)]
    replace: bool,
}

async fn publish_keypackages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PublishKpReq>,
) -> Result<StatusCode, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    // 限流：单次最多 100 包 × (CBOR 解析 + DB INSERT)，非好友批量可耗尽 CPU
    // 与 SQLite 写锁。0.2/s burst 3 = 突发 3 次，稳态 5 秒一次。
    if !state.check_rate_scoped(&uid, "publish_kp", 0.2, 3.0) {
        state.metrics.rate_limited();
        return Err(ApiError::TooManyRequests(
            "keypackage publish rate limited".into(),
        ));
    }

    if req.packages.is_empty() || req.packages.len() > 100 {
        return Err(ApiError::BadRequest("packages count 1..=100".into()));
    }

    let now = db::now_ms();
    // 身份重建：旧 KP 属于已销毁的本地身份，先整体清空，避免 host 取到
    // stale KP 后 Welcome/DM 报 NoMatchingKeyPackage。
    if req.replace {
        sqlx::query("DELETE FROM key_packages WHERE login_id = ?")
            .bind(uid.as_str())
            .execute(&state.pool)
            .await?;
    }
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
#[allow(dead_code)]
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
    if !cfg!(feature = "dev-mode") && !state.check_rate_scoped(&uid, "kp_fetch", 1.0, 2.0) {
        state.metrics.rate_limited();
        return Err(ApiError::TooManyRequests(
            "keypackage fetch rate limited".into(),
        ));
    }

    // 防抽干第一层（补充）：单个请求者对同一 target 的消费速率上限
    // （约 1 次 / 10s，突发 2）。即使攻击者换账号，单账号也无法快速清空队列。
    if !cfg!(feature = "dev-mode") {
        let pair_scope = format!("kp_pair:{}:{}", uid.as_str(), login_id);
        if !state.check_rate_key(&pair_scope, 0.1, 2.0) {
            state.metrics.rate_limited();
            return Err(ApiError::TooManyRequests(
                "keypackage pair rate limited".into(),
            ));
        }
    }

    // 好友（双向 accepted）不受目标预算与保留量限制。
    // 好友关系必须对方接受才能建立，攻击者无法单方面获取该豁免。
    // dev-mode 下跳过好友判定（测试用）；生产环境走真实好友关系
    let is_friend = if cfg!(feature = "dev-mode") {
        true
    } else {
        crate::friends::is_friend(&state.pool, uid.as_str(), &login_id)
            .await
            .map_err(|e| ApiError::Internal(format!("db: {}", e)))?
    };

    if !is_friend {
        // 防抽干第二层：per-target 全局限流（防多账号轮换）。
        if !cfg!(feature = "dev-mode") {
            let target_scope = format!("kp_target:{}", login_id);
            if !state.check_rate_key(&target_scope, 0.1, 2.0) {
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
        if !cfg!(feature = "dev-mode") && remaining < 8 {
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

    // P1: kick old sessions — 同一账号只保留最新一个 session
    // 多设备/多 tab 同时在线会破坏 MLS identity 的 ratchet 状态
    // （每个设备独立建 group，导致 epoch 分叉）
    // 简单方案：新 login 踢掉同 login_id 的所有旧 session
    if let Err(e) = sqlx::query("DELETE FROM sessions WHERE login_id = ? AND token != ?")
        .bind(login_id)
        .bind(db::hash_token(&token))
        .execute(&state.pool)
        .await
    {
        tracing::warn!("kick old sessions failed: {}", e);
    }
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

/// 可信反向代理列表。`KAEDE_TRUSTED_PROXIES` 为逗号分隔的 IP / CIDR。
///
/// 默认只信任 loopback（本机 Caddy/Nginx/SSH 隧道）。容器部署时反向代理位于
/// Docker bridge，必须显式把它所在的网段加入该变量，否则所有用户会被归并成
/// 同一个代理 IP —— 登录失败计数与注册限流会变成全站共享。
///
/// **不要**默认信任整个 RFC1918：同 VPC 内攻击者/SSRF 可以伪造 XFF 绕过
/// 基于 `(uid, ip)` 的登录限流。
fn trusted_proxies() -> &'static [ipnet::IpNet] {
    static TRUSTED: std::sync::OnceLock<Vec<ipnet::IpNet>> = std::sync::OnceLock::new();
    TRUSTED.get_or_init(|| {
        let raw = std::env::var("KAEDE_TRUSTED_PROXIES").unwrap_or_default();
        let mut nets: Vec<ipnet::IpNet> = Vec::new();
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match part.parse::<ipnet::IpNet>() {
                Ok(net) => nets.push(net),
                // 启动期 Config::validate 已经 fail-fast；这里只是防御性兜底。
                Err(e) => tracing::error!(
                    "ignoring invalid KAEDE_TRUSTED_PROXIES entry {:?}: {}",
                    part,
                    e
                ),
            }
        }
        if nets.is_empty() {
            nets.push("127.0.0.0/8".parse().expect("static loopback cidr"));
            nets.push("::1/128".parse().expect("static loopback cidr"));
        }
        nets
    })
}

/// 启动期校验 `KAEDE_TRUSTED_PROXIES`。
///
/// 任一条目非法都必须 fail-fast：静默丢弃会让所有用户被归并成同一个
/// 代理 IP（登录失败计数/注册限流全站共享），也会让伪造 XFF 的同网段
/// 攻击者获得信任。返回合法条目数（空串 → 0，表示只用 loopback 默认值）。
pub(crate) fn validate_trusted_proxies(raw: &str) -> Result<usize, String> {
    let mut count = 0;
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        part.parse::<ipnet::IpNet>()
            .map_err(|e| format!("invalid KAEDE_TRUSTED_PROXIES entry {:?}: {}", part, e))?;
        count += 1;
    }
    Ok(count)
}

fn is_trusted_proxy(ip: std::net::IpAddr) -> bool {
    trusted_proxies().iter().any(|net| net.contains(&ip))
}

/// 取真实客户端 IP。
///
/// - peer 不在可信代理列表 → 直接用 peer，忽略任何转发头。
/// - peer 是可信代理 → 从 `X-Forwarded-For` **右往左**跳过可信代理，
///   第一个不可信地址就是客户端。取第一个等于信任客户端自己塞的值。
/// - 整条链都可信（伪造 loopback）→ 退回 peer，避免被伪造成本机。
pub(crate) fn client_ip(headers: &HeaderMap, peer: std::net::SocketAddr) -> std::net::IpAddr {
    let peer_ip = peer.ip();
    if !is_trusted_proxy(peer_ip) {
        return peer_ip;
    }
    let chain: Vec<std::net::IpAddr> = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .filter_map(|part| part.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default();
    for ip in chain.iter().rev() {
        if !is_trusted_proxy(*ip) {
            return *ip;
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok())
        .filter(|ip| !is_trusted_proxy(*ip))
        .unwrap_or(peer_ip)
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

#[derive(Serialize)]
struct PostMessageResp {
    ts: i64,
}

async fn post_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<PostMessageResp>, ApiError> {
    // C1：用 auth uid 覆盖客户端自报的 sender，防止冒充
    let uid = authenticate(&state, &headers).await?;
    tracing::info!(
        "[post_message] uid={} channel={} ct_len={}",
        uid.as_str(),
        req.channel_id,
        req.ciphertext.len()
    );

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

    // 【频道隔离核心 + 时序修复】实时推送给 channel_members ∪ online peers
    // 原因：新加入的成员 HTTP join_channel 可能还在路上，
    //   此时它不在 channel_members 里，会漏收消息。
    // 安全性：ChannelMessage 是 AES-GCM 加密的，非成员收到无法解密（client 端 drop）。
    if true {
        // 1. 查 channel_members
        let db_members: Vec<(String,)> = sqlx::query_as(
            "SELECT login_id FROM channel_members WHERE channel_id = ? AND login_id != ?",
        )
        .bind(&req.channel_id)
        .bind(uid.as_str())
        .fetch_all(&state.pool)
        .await?;

        // 2. channel_members ∪ online peers（排除发送者）
        let mut recipients: std::collections::HashSet<String> =
            db_members.into_iter().map(|(m,)| m).collect();
        for peer_uid in state.peer_uids() {
            if peer_uid.as_str() == uid.as_str() {
                continue;
            }
            recipients.insert(peer_uid.0);
        }

        tracing::info!(
            "[post_message] channel={} recipients={} (peers only)",
            req.channel_id,
            recipients.len()
        );

        if !state.try_reserve_broadcast(ct.len(), recipients.len()) {
            state.metrics.broadcast_budget_rejected();
            tracing::warn!(
                "channel fanout budget exceeded (channel={}, bytes={}, recipients={}); persisted only",
                req.channel_id,
                ct.len(),
                recipients.len()
            );
            return Ok(Json(PostMessageResp { ts }));
        }

        for member in recipients {
            let target = UserId::new(member);
            let delivered = state.send_to(
                &target,
                ServerMessage::ChannelMessage {
                    channel_id: req.channel_id.clone(),
                    from: uid.clone(),
                    ts,
                    ciphertext: ct.clone(),
                },
            );
            if !delivered {
                let mut pending = Vec::with_capacity(8 + ct.len());
                pending.extend_from_slice(&ts.to_be_bytes());
                pending.extend_from_slice(&ct);
                if let Err(e) = db::enqueue_envelope(
                    &state.pool,
                    target.as_str(),
                    uid.as_str(),
                    "channel_message",
                    Some(&req.channel_id),
                    &pending,
                )
                .await
                {
                    tracing::warn!("enqueue channel_message for {} failed: {}", target, e);
                }
            }
        }
    }

    // 返回服务器权威 ts：客户端用它落本地“自己发出的消息”，
    // 否则重载时本地客户端 ts 与服务端 ts 不一致会产生重复消息。
    Ok(Json(PostMessageResp { ts }))
}

async fn get_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<GetMessagesQuery>,
) -> Result<Json<MessagesResp>, ApiError> {
    let uid = authenticate(&state, &headers).await?;

    // 历史读取限流：单次响应已按字节封顶，仍需防止并发请求反复分配大缓冲。
    if !state.check_rate_scoped(&uid, "history_read", 2.0, 10.0) {
        state.metrics.rate_limited();
        return Err(ApiError::TooManyRequests(
            "history read rate limited".into(),
        ));
    }
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
        "WITH recent AS ( \
           SELECT ts, sender, ciphertext, \
                  SUM(LENGTH(ciphertext)) OVER (ORDER BY ts DESC) AS run_bytes \
           FROM ( \
             SELECT ts, sender, ciphertext FROM messages \
             WHERE channel_id = ? AND ts < ? \
               AND (? = 0 OR substr(ciphertext, 1, 1) = X'03') \
             ORDER BY ts DESC LIMIT ? \
           ) \
         ) \
         SELECT ts, sender, ciphertext FROM recent \
         WHERE run_bytes <= ? ORDER BY ts ASC",
    )
    .bind(&channel_id)
    .bind(before)
    .bind(is_private_raw)
    .bind(limit)
    .bind(crate::db::MAX_HISTORY_RESPONSE_BYTES)
    .fetch_all(&state.pool)
    .await?;

    // SQL 已按时间升序返回
    let out: Vec<MessageRecord> = rows
        .into_iter()
        .map(|(ts, sender, ct)| MessageRecord {
            ts,
            sender,
            ciphertext: base64_url(&ct),
        })
        .collect();

    Ok(Json(MessagesResp { messages: out }))
}

#[derive(Deserialize)]
struct GetMessagesQuery {
    before_ts: Option<i64>,
    limit: Option<i64>,
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn headers(xff: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_str(xff).unwrap());
        h
    }

    fn peer(ip: &str) -> std::net::SocketAddr {
        std::net::SocketAddr::new(ip.parse().unwrap(), 443)
    }

    #[test]
    fn untrusted_peer_ignores_forwarded_headers() {
        assert_eq!(
            client_ip(&headers("203.0.113.9"), peer("198.51.100.7")),
            "198.51.100.7".parse::<std::net::IpAddr>().unwrap()
        );
    }

    #[test]
    fn loopback_proxy_uses_rightmost_untrusted_hop() {
        // 左边是客户端伪造的地址，右边才是代理看到的直连地址。
        assert_eq!(
            client_ip(&headers("1.2.3.4, 203.0.113.9"), peer("127.0.0.1")),
            "203.0.113.9".parse::<std::net::IpAddr>().unwrap()
        );
    }

    #[test]
    fn spoofed_trusted_hop_falls_back_to_peer() {
        assert_eq!(
            client_ip(&headers("127.0.0.1"), peer("127.0.0.1")),
            "127.0.0.1".parse::<std::net::IpAddr>().unwrap()
        );
    }
}

