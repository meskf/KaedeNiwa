const fs = require('fs')
const log = []

// ===== 1. http.rs: dev-mode 默认 op3 =====
let s = fs.readFileSync('crates/kaede-server/src/http.rs', 'utf8')
const oldReg = `    // 同步写 members 表。P1-9：用 SQL 子查询原子判 op5，消除并发注册竞态。
    sqlx::query(
        "INSERT INTO members (login_id, nickname, op, bio, title, avatar_url, joined_at, updated_at) \\
         VALUES (?, ?, (SELECT CASE WHEN COUNT(*) = 0 THEN 'op5' ELSE 'op2' END FROM members), '', '', '', ?, ?)",
    )`
const newReg = `    // 同步写 members 表。P1-9：用 SQL 子查询原子判 op5，消除并发注册竞态。
    //
    // dev-mode：非首个用户默认 op3（普管），方便本地测试创建/删除频道。
    // 生产构建（无 dev-mode feature）：非首个用户默认 op2（成员）。
    let op_subquery = if cfg!(feature = "dev-mode") {
        "(SELECT CASE WHEN COUNT(*) = 0 THEN 'op5' ELSE 'op3' END FROM members)"
    } else {
        "(SELECT CASE WHEN COUNT(*) = 0 THEN 'op5' ELSE 'op2' END FROM members)"
    };
    sqlx::query(&format!(
        "INSERT INTO members (login_id, nickname, op, bio, title, avatar_url, joined_at, updated_at) \\
         VALUES (?, ?, {}, '', '', '', ?, ?)",
        op_subquery
    ))`
if (!s.includes(oldReg)) { console.error('FAIL: register SQL'); process.exit(1) }
s = s.replace(oldReg, newReg)
fs.writeFileSync('crates/kaede-server/src/http.rs', s)
log.push('OK: http.rs dev-mode op3')

// ===== 2. promote bin =====
const promote = `//! 提权工具：给指定用户设置 op 级别。
//!
//! 用法：
//!   cargo run -p kaede-server --bin promote -- <login_id> <op>
//!
//! 示例：
//!   cargo run -p kaede-server --bin promote -- alice op5
//!   cargo run -p kaede-server --bin promote -- bob op3
//!
//! op 级别：
//!   op1 = 访客
//!   op2 = 成员（默认）
//!   op3 = 普管（可创建/删除频道、分类）
//!   op4 = 超管
//!   op5 = 服主

use anyhow::{Result, bail};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        bail!("usage: {} <login_id> <op1..op5>", args[0]);
    }
    let login_id = &args[1];
    let op = &args[2];
    if !matches!(op.as_str(), "op1" | "op2" | "op3" | "op4" | "op5") {
        bail!("op must be one of op1, op2, op3, op4, op5");
    }

    // 复用 server 的 db URL 解析：优先 KAEDE_DB_URL，否则默认 sqlite://kaede.db
    let db_url = std::env::var("KAEDE_DB_URL")
        .unwrap_or_else(|_| "sqlite://kaede.db?mode=rwc".to_string());

    println!("db: {}", db_url);
    println!("promoting '{}' -> {}", login_id, op);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await?;

    let affected = sqlx::query("UPDATE members SET op = ?, updated_at = ? WHERE login_id = ?")
        .bind(op)
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(login_id)
        .execute(&pool)
        .await?
        .rows_affected();

    if affected == 0 {
        bail!("login_id '{}' not found in members table", login_id);
    }

    // 输出当前分布，方便确认
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT login_id, op FROM members ORDER BY joined_at"
    )
    .fetch_all(&pool)
    .await?;
    println!("\ncurrent members:");
    for (id, o) in rows {
        let marker = if id == *login_id { " <- updated" } else { "" };
        println!("  {} = {}{}", id, o, marker);
    }
    Ok(())
}
`
fs.writeFileSync('crates/kaede-server/src/bin/promote.rs', promote)
log.push('OK: bin/promote.rs')

// ===== 3. Cargo.toml: 添加 bin 条目 =====
let cargo = fs.readFileSync('crates/kaede-server/Cargo.toml', 'utf8')
if (!cargo.includes('name = "promote"')) {
  const newBin = cargo.replace(
    `[[bin]]
name = "kaede-server"
path = "src/main.rs"`,
    `[[bin]]
name = "kaede-server"
path = "src/main.rs"

[[bin]]
name = "promote"
path = "src/bin/promote.rs"`
  )
  fs.writeFileSync('crates/kaede-server/Cargo.toml', newBin)
  log.push('OK: Cargo.toml promote bin')
} else {
  log.push('SKIP: promote bin already in Cargo.toml')
}

// ===== 4. 检查 chrono 是否已是 dependency =====
if (!cargo.includes('chrono')) {
  console.log('WARN: chrono not in Cargo.toml — promote.rs uses chrono::Utc, may need to use db::now_ms instead')
}

log.forEach(l => console.log(l))
console.log('DONE')
