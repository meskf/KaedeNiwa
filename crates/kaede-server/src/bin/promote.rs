//! 提权工具：给指定用户设置 op 级别。
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

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
        .bind(now_ms())
        .bind(login_id)
        .execute(&pool)
        .await?
        .rows_affected();

    if affected == 0 {
        bail!("login_id '{}' not found in members table", login_id);
    }

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
