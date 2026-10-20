-- 私密频道哈希强度档位（仅 UI 展示用；verify 走 PHC 串自带的真实参数）
ALTER TABLE channels ADD COLUMN hash_tier TEXT;
