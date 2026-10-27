const fs = require('fs')
const p = 'crates/kaede-server/src/channels.rs'
let s = fs.readFileSync(p, 'utf8')
const log = []
function rep(from, to, name) {
  if (!s.includes(from)) { log.push('MISS: ' + name); return }
  s = s.replace(from, to)
  log.push('OK: ' + name)
}

// CreateChannelReq: add category_id
rep(
  `    /// 哈希强度档位："19"（默认）/ "64" / "256"。
    #[serde(default)]
    hash_tier: Option<String>,
}`,
  `    /// 哈希强度档位："19"（默认）/ "64" / "256"。
    #[serde(default)]
    hash_tier: Option<String>,
    /// 所属分类（categories.id）。None = 未分类。
    #[serde(default)]
    category_id: Option<String>,
}`,
  'CreateChannelReq.category_id')

// UpdateChannelReq: add category_id
rep(
  `struct UpdateChannelReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    position: Option<i64>,
}`,
  `struct UpdateChannelReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    position: Option<i64>,
    #[serde(default)]
    category_id: Option<String>,
}`,
  'UpdateChannelReq.category_id')

// list_channels query: add category_id
rep(
  `    type ChannelRow = (String, String, String, i64, i64, i64, Option<String>);
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, name, kind, position, created_at, is_private, hash_tier \\
         FROM channels \\
         WHERE is_private = 0 \\
            OR id IN (SELECT channel_id FROM channel_members WHERE login_id = ?) \\
         ORDER BY position, id",
    )`,
  `    type ChannelRow = (String, String, String, i64, i64, i64, Option<String>, Option<String>);
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, name, kind, position, created_at, is_private, hash_tier, category_id \\
         FROM channels \\
         WHERE is_private = 0 \\
            OR id IN (SELECT channel_id FROM channel_members WHERE login_id = ?) \\
         ORDER BY position, id",
    )`,
  'list_channels query')

// list_channels map: add category_id
rep(
  `            |(id, name, kind, position, created_at, is_private, hash_tier)| Channel {
                id,
                name,
                kind,
                position,
                created_at,
                is_private: is_private != 0,
                hash_tier,
            },`,
  `            |(id, name, kind, position, created_at, is_private, hash_tier, category_id)| Channel {
                id,
                name,
                kind,
                position,
                created_at,
                is_private: is_private != 0,
                hash_tier,
                category_id,
            },`,
  'list_channels map')

// create_channel INSERT: add category_id column
rep(
  `    sqlx::query(
        "INSERT INTO channels (id, name, kind, position, created_at, is_private, password_hash, hash_tier) \\
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&req.kind)
    .bind(position)
    .bind(now)
    .bind(if req.is_private { 1i64 } else { 0i64 })
    .bind(password_hash.as_deref())
    .bind(hash_tier.as_deref())`,
  `    sqlx::query(
        "INSERT INTO channels (id, name, kind, position, created_at, is_private, password_hash, hash_tier, category_id) \\
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&req.kind)
    .bind(position)
    .bind(now)
    .bind(if req.is_private { 1i64 } else { 0i64 })
    .bind(password_hash.as_deref())
    .bind(hash_tier.as_deref())
    .bind(req.category_id.as_deref())`,
  'create_channel INSERT')

// create_channel Channel struct: add category_id
rep(
  `    let channel = Channel {
        id,
        name,
        kind: req.kind,
        position,
        created_at: now,
        is_private: req.is_private,
        hash_tier: hash_tier.clone(),
    };`,
  `    let channel = Channel {
        id,
        name,
        kind: req.kind,
        position,
        created_at: now,
        is_private: req.is_private,
        hash_tier: hash_tier.clone(),
        category_id: req.category_id.clone(),
    };`,
  'create_channel struct')

// update_channel SELECT: add category_id
rep(
  `    let row: Option<(String, String, i64, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT name, kind, position, created_at, is_private, hash_tier FROM channels WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?;
    let (mut name, kind, mut position, created_at, is_private_raw, hash_tier) =
        row.ok_or_else(|| ApiError::NotFound("channel".into()))?;`,
  `    let row: Option<(String, String, i64, i64, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT name, kind, position, created_at, is_private, hash_tier, category_id FROM channels WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?;
    let (mut name, kind, mut position, created_at, is_private_raw, hash_tier, mut category_id) =
        row.ok_or_else(|| ApiError::NotFound("channel".into()))?;`,
  'update_channel SELECT')

// update_channel: handle category_id update (after position block)
rep(
  `    if let Some(p) = req.position {
        // M28: 上限校验，防 create_channel 的 MAX(position)+1 溢出
        if !(0..=i32::MAX as i64).contains(&p) {
            return Err(ApiError::BadRequest("position must be 0..=i32::MAX".into()));
        }
        position = p;
        sqlx::query("UPDATE channels SET position = ? WHERE id = ?")
            .bind(position)
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }

    let channel = Channel {
        id,
        name,
        kind,
        position,
        created_at,
        is_private,
        hash_tier,
    };`,
  `    if let Some(p) = req.position {
        // M28: 上限校验，防 create_channel 的 MAX(position)+1 溢出
        if !(0..=i32::MAX as i64).contains(&p) {
            return Err(ApiError::BadRequest("position must be 0..=i32::MAX".into()));
        }
        position = p;
        sqlx::query("UPDATE channels SET position = ? WHERE id = ?")
            .bind(position)
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }

    if req.category_id.is_some() {
        // Treat empty string as null (uncategorize).
        let new_cat = req.category_id.clone().filter(|s| !s.is_empty());
        sqlx::query("UPDATE channels SET category_id = ? WHERE id = ?")
            .bind(new_cat.as_deref())
            .bind(&id)
            .execute(&state.pool)
            .await?;
        category_id = new_cat;
    }

    let channel = Channel {
        id,
        name,
        kind,
        position,
        created_at,
        is_private,
        hash_tier,
        category_id,
    };`,
  'update_channel category_id + struct')

fs.writeFileSync(p, s)
log.forEach(l => console.log(l))
