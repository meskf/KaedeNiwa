-- nodes 表此前只由 db::init_nodes_table() 内联创建，未进 migrations。
-- 后果：任何绕过 main.rs 初始化顺序（db::init -> init_nodes_table -> ensure_default_channels）
-- 的路径（如 spawn_test_server、ws::mk_state）会在缺表状态下跑 SELECT/INSERT FROM nodes，
-- 触发 SQLITE_ERROR "no such table: nodes"，HTTP 500。
--
-- 本 migration 把建表纳入 migrations，与 init_nodes_table() 幂等共存：
-- - 全新库：migration 建空表 -> init_nodes_table 的 CREATE IF NOT EXISTS 跳过
-- - 老库：CREATE IF NOT EXISTS 跳过（nodes 表已存在），init_nodes_table 继续做数据同步
-- - 不跑 migration 的测试辅助（ws::mk_state 用裸 SqlitePoolOptions 直连）：由 init_nodes_table 建表兜底
--
-- 不把 INSERT ... FROM channels 数据同步逻辑放进 migration：
-- 那是每次启动都要跑的幂等同步（兼容老版本 channels 数据），语义上属于 init_nodes_table。

CREATE TABLE IF NOT EXISTS nodes (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    parent_id     TEXT,
    kind          TEXT NOT NULL,
    position      INTEGER NOT NULL DEFAULT 0,
    is_private    INTEGER NOT NULL DEFAULT 0,
    password_hash TEXT,
    hash_tier     TEXT,
    created_at    INTEGER NOT NULL
);
