const sqlite3 = require('sqlite3')
const db = new sqlite3.Database('kaede.db')
db.all("SELECT name FROM sqlite_master WHERE type='table' AND name IN ('categories', 'channels')", (e, rows) => {
  if (e) { console.error('ERR tables:', e); process.exit(1) }
  console.log('tables:', rows.map(r => r.name).join(', '))
  db.all("SELECT id, name, position FROM categories ORDER BY position", (e, cats) => {
    if (e) { console.error('ERR cats:', e); process.exit(1) }
    console.log('categories:', JSON.stringify(cats))
    db.all("SELECT id, name, kind, category_id FROM channels ORDER BY position LIMIT 5", (e, chs) => {
      if (e) { console.error('ERR chs:', e); process.exit(1) }
      console.log('channels:', JSON.stringify(chs))
      db.close()
    })
  })
})
