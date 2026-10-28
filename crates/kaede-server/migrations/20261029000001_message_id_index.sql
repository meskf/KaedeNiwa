-- 增量拉取支持：客户端知道 last_id 后只拿 id > last_id 的消息。
-- 现有 idx_msg_channel(channel_id, ts) 对 id > ? 查询不最优，
-- 新增 (channel_id, id) 索引专门服务增量拉取路径。
CREATE INDEX IF NOT EXISTS idx_msg_channel_id ON messages(channel_id, id);
