-- DM 会话索引表：避免 list_conversations 全表扫描 messages。
-- 迁移时从现有 messages 回填一次。

CREATE TABLE IF NOT EXISTS dm_conversations (
    conv_id TEXT PRIMARY KEY,
    a       TEXT NOT NULL,
    b       TEXT NOT NULL,
    last_ts INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_dm_a_ts ON dm_conversations(a, last_ts DESC);
CREATE INDEX IF NOT EXISTS idx_dm_b_ts ON dm_conversations(b, last_ts DESC);

INSERT OR IGNORE INTO dm_conversations (conv_id, a, b, last_ts)
SELECT channel_id,
       substr(channel_id, 4, instr(substr(channel_id, 4), ':') - 1),
       substr(channel_id, 4 + instr(substr(channel_id, 4), ':')),
       MAX(ts)
FROM messages
WHERE channel_id LIKE 'dm:%'
  AND instr(substr(channel_id, 4), ':') > 1
GROUP BY channel_id;
