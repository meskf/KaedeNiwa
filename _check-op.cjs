const BASE = 'http://127.0.0.1:18080'
async function main() {
  const u = 'opcheck' + Math.random().toString(36).slice(2, 8)
  const reg = await fetch(BASE + '/api/register', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ login_id: u, password: 'password123' })
  })
  const rd = await reg.json()
  console.log('NEW_USER_OP', JSON.stringify(rd))
  // 尝试创建
  const token = rd.token || rd.session_token
  const c = await fetch(BASE + '/api/channels', {
    method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + token },
    body: JSON.stringify({ name: 'test-create-' + u, kind: 'text', category_id: 'cat_text' })
  })
  console.log('CREATE_STATUS', c.status)
  console.log('CREATE_BODY', await c.text())
}
main().catch(e => { console.error(e); process.exit(1) })
