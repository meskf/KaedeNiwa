-- 每个账号的消息密文累计字节数（终身配额）。
--
-- 消息本身没有附件那样的 32 MiB 单文件上限；没有配额时，一个账号
-- 可以按 4 MiB × 10 条/秒持续写入，快速耗尽磁盘。
CREATE TABLE IF NOT EXISTS message_usage (
    login_id TEXT PRIMARY KEY,
    bytes    INTEGER NOT NULL DEFAULT 0
);
