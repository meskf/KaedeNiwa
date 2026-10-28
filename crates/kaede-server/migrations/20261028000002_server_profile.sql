CREATE TABLE IF NOT EXISTS server_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL DEFAULT 'Kaede Server',
    description TEXT NOT NULL DEFAULT 'Workspace',
    icon_url TEXT,
    updated_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO server_profile (id, name, description, updated_at) 
VALUES (1, 'Kaede Server', 'Workspace', CAST(strftime('%s', 'now') AS INTEGER) * 1000);
