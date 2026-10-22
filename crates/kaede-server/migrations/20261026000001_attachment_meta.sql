-- 附件元数据：用于每用户配额统计 + 频道删除时回收密文 blob。
-- 服务端只能看到不透明的密文和上传上下文（频道/DM id），看不到内容。

CREATE TABLE IF NOT EXISTS attachments (
    id         TEXT PRIMARY KEY,
    owner      TEXT NOT NULL,
    target_id  TEXT,
    size       INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_attachments_owner ON attachments(owner);
CREATE INDEX IF NOT EXISTS idx_attachments_target ON attachments(target_id);
