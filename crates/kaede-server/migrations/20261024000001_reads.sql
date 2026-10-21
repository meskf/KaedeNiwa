-- 已读位置：按用户记录频道/DM 的最后已读 ts。
-- 未读 = messages.ts > last_read_ts AND sender != 自己。

CREATE TABLE IF NOT EXISTS channel_reads (
    login_id     TEXT NOT NULL,
    channel_id   TEXT NOT NULL,
    last_read_ts INTEGER NOT NULL,
    PRIMARY KEY (login_id, channel_id)
);

CREATE TABLE IF NOT EXISTS dm_reads (
    login_id     TEXT NOT NULL,
    conv_id      TEXT NOT NULL,
    last_read_ts INTEGER NOT NULL,
    PRIMARY KEY (login_id, conv_id)
);
