-- 管理操作审计：所有面板写操作记录在这里。
-- 读操作（dashboard/users/channels/logs）不记，避免噪音。
CREATE TABLE IF NOT EXISTS admin_audit (
    id     INTEGER PRIMARY KEY AUTOINCREMENT,
    ts     INTEGER NOT NULL,
    admin  TEXT    NOT NULL,
    action TEXT    NOT NULL,
    target TEXT,
    result TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_admin_audit_ts ON admin_audit(ts DESC);
