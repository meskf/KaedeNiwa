-- 最近的主群 Commit，用于离线成员 epoch 追赶。
-- 客户端 Join 后上报自己的 epoch，服务器补投 epoch 更大的 Commit。

CREATE TABLE IF NOT EXISTS mls_commits (
    epoch      INTEGER PRIMARY KEY,
    from_uid   TEXT NOT NULL,
    payload    BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_mls_commits_epoch ON mls_commits(epoch);
