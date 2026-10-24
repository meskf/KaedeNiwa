-- 主群 mls_commits 与频道 Commit 共享表，通过 channel_id 区分。
-- 主群：channel_id = ''
-- 频道：channel_id = <channel id>
-- 重建表以支持复合主键 (channel_id, epoch)，避免不同 channel 的 epoch 冲突。

CREATE TABLE mls_commits_new (
    channel_id TEXT NOT NULL DEFAULT '',
    epoch INTEGER NOT NULL,
    from_uid TEXT NOT NULL,
    payload BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (channel_id, epoch)
);

INSERT INTO mls_commits_new (channel_id, epoch, from_uid, payload, created_at)
SELECT '', epoch, from_uid, payload, created_at FROM mls_commits;

DROP TABLE mls_commits;

ALTER TABLE mls_commits_new RENAME TO mls_commits;

CREATE INDEX IF NOT EXISTS idx_mls_commits_channel_epoch
    ON mls_commits(channel_id, epoch);
