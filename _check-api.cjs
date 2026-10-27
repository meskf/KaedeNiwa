const BASE = 'http://127.0.0.1:18080'
const u = 'catcheck' + Math.random().toString(36).slice(2, 8)
async function main() {
  const reg = await fetch(BASE + '/api/register', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ login_id: u, password: 'password123' })
  })
  const rd = await reg.json()
  const token = rd.token || rd.session_token
  if (!token) { console.error('REG FAIL', rd); process.exit(1) }
  console.log('REGISTERED', u)

  const cats = await fetch(BASE + '/api/categories', { headers: { Authorization: 'Bearer ' + token } })
  console.log('CATEGORIES_STATUS', cats.status)
  console.log('CATEGORIES', JSON.stringify(await cats.json(), null, 2))

  const chs = await fetch(BASE + '/api/channels', { headers: { Authorization: 'Bearer ' + token } })
  const chsData = await chs.json()
  console.log('CHANNELS', JSON.stringify((chsData.channels || []).map(c => ({ id: c.id, name: c.name, kind: c.kind, category_id: c.category_id })), null, 2))
}
main().catch(e => { console.error('FATAL', e); process.exit(1) })
