-- 已删除账号的 login_id 墓碑。
--
-- 删除成员会删除 accounts 行；如果没有墓碑，该 login_id 会立即变成
-- “未注册”，客户端自动注册或攻击者手动注册都能把账号重新建回来，
-- 使管理员的删除操作失效。
CREATE TABLE IF NOT EXISTS retired_logins (
    login_id   TEXT PRIMARY KEY,
    retired_at INTEGER NOT NULL
);
