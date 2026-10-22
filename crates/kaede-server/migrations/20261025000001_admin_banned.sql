-- 管理面板：账号封禁标志。
-- 封禁后：新登录被拒 + 已有 session 全部清空；WS 长连接在下次心跳时断开。
ALTER TABLE accounts ADD COLUMN banned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE accounts ADD COLUMN banned_at INTEGER;
ALTER TABLE accounts ADD COLUMN banned_reason TEXT;
