-- Categories: group channels into collapsible sections (Discord-style).
-- A channel belongs to at most one category. NULL = uncategorized fallback.
CREATE TABLE IF NOT EXISTS categories (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_categories_position ON categories(position);

-- Nullable FK; no ON DELETE cascade (SQLite doesn't enforce FK by default).
ALTER TABLE channels ADD COLUMN category_id TEXT;
CREATE INDEX IF NOT EXISTS idx_channels_category ON channels(category_id);

-- Seed default categories if not present, then backfill existing channels.
INSERT INTO categories (id, name, position, created_at)
SELECT 'cat_text', 'Text Channels', 0, CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE NOT EXISTS (SELECT 1 FROM categories WHERE id = 'cat_text');

INSERT INTO categories (id, name, position, created_at)
SELECT 'cat_voice', 'Voice Channels', 1, CAST(strftime('%s','now') AS INTEGER) * 1000
WHERE NOT EXISTS (SELECT 1 FROM categories WHERE id = 'cat_voice');

UPDATE channels SET category_id = 'cat_text' WHERE kind = 'text' AND category_id IS NULL;
UPDATE channels SET category_id = 'cat_voice' WHERE kind = 'voice' AND category_id IS NULL;
