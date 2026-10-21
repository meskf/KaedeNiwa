-- 会话管理：设备名、创建/最近活跃时间、对外可见的 session_id。
-- token 仍只存 SHA-256；session_id 是随机公开标识，仅用于列表/撤销。

ALTER TABLE sessions ADD COLUMN device_name  TEXT    NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN created_at   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN last_seen_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN session_id   TEXT    NOT NULL DEFAULT '';

-- 为旧会话补一个随机 id 与时间（SQLite 随机 8 字节 → 16 hex）。
UPDATE sessions
SET session_id = lower(hex(randomblob(8))),
    created_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    last_seen_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE session_id = '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_sid ON sessions(session_id);
