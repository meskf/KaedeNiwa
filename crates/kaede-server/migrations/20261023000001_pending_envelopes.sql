-- 离线 MLS 信封（Welcome / ChannelSync）。
-- 作用：目标离线时服务器暂存，目标下次 Join 时按入队顺序补投。
-- 只存密文载荷，服务器不解析。

CREATE TABLE IF NOT EXISTS pending_envelopes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    to_uid     TEXT NOT NULL,
    from_uid   TEXT NOT NULL,
    kind       TEXT NOT NULL,           -- 'mls_welcome' | 'channel_sync'
    channel_id TEXT,                    -- channel_sync 用
    payload    BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pending_to ON pending_envelopes(to_uid, id);
