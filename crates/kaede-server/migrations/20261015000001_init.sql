-- KaedeNiwa server initial schema.
-- 
-- 说明：使用 IF NOT EXISTS 是为了兼容「已经跑过内联 schema 的旧 DB」。
-- sqlx::migrate! 首次运行时会把本文件记为 version 1 已应用；
-- 之后新加字段请新建 migration 文件，不要改这个。

CREATE TABLE IF NOT EXISTS accounts (
    login_id      TEXT PRIMARY KEY,
    password_hash TEXT NOT NULL,
    nickname      TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token      TEXT PRIMARY KEY,
    login_id   TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_login ON sessions(login_id);

CREATE TABLE IF NOT EXISTS key_packages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    login_id   TEXT NOT NULL,
    package    BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_kp_login ON key_packages(login_id);

CREATE TABLE IF NOT EXISTS messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id TEXT NOT NULL,
    ts         INTEGER NOT NULL,
    sender     TEXT NOT NULL,
    ciphertext BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_msg_channel ON messages(channel_id, ts);

CREATE TABLE IF NOT EXISTS channels (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_channels_pos ON channels(position);

CREATE TABLE IF NOT EXISTS members (
    login_id   TEXT PRIMARY KEY,
    nickname   TEXT NOT NULL,
    op         TEXT NOT NULL DEFAULT 'op2',
    bio        TEXT NOT NULL DEFAULT '',
    title      TEXT NOT NULL DEFAULT '',
    avatar_url TEXT NOT NULL DEFAULT '',
    joined_at  INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS friends (
    login_id        TEXT NOT NULL,
    friend_login_id TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (login_id, friend_login_id)
);
CREATE INDEX IF NOT EXISTS idx_friends_login ON friends(login_id);