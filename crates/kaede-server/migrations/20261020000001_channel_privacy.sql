-- KaedeNiwa 频道私密化（B 方案第一步：服务端准入 + 密码校验）。
-- 真正的密码学隔离（频道级 MLS group）在客户端实现；本迁移只做 server 侧准入控制。

ALTER TABLE channels ADD COLUMN is_private INTEGER NOT NULL DEFAULT 0;
ALTER TABLE channels ADD COLUMN password_hash TEXT;

CREATE TABLE IF NOT EXISTS channel_members (
    channel_id TEXT NOT NULL,
    login_id   TEXT NOT NULL,
    joined_at  INTEGER NOT NULL,
    PRIMARY KEY (channel_id, login_id)
);
CREATE INDEX IF NOT EXISTS idx_channel_members_uid ON channel_members(login_id);