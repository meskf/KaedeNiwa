const { chromium } = require('playwright');
(async () => {
  const name = `diag-${Date.now().toString(36)}`;
  const browser = await chromium.launch({ headless: true });
  const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
  const page = await ctx.newPage();

  const non2xx = [];
  const consoleMsgs = [];

  page.on('response', async (res) => {
    const s = res.status();
    if (s >= 400) {
      let body = '';
      try { body = (await res.text()).slice(0, 200); } catch {}
      non2xx.push({
        status: s,
        method: res.request().method(),
        url: res.url(),
        resourceType: res.request().resourceType(),
        body,
      });
    }
  });

  page.on('console', async (m) => {
    if (m.type() !== 'error') return;
    let args = [];
    try {
      args = await Promise.all(m.args().map(a => a.jsonValue().catch(() => '<unserializable>')));
    } catch {}
    consoleMsgs.push({ text: m.text(), args });
  });

  await page.goto('http://127.0.0.1:5173/', { waitUntil: 'networkidle', timeout: 30000 });
  await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 15000 });
  await page.getByTestId('login-id').fill(name);
  await page.getByTestId('login-password').fill('password123');
  const reg = page.getByTestId('login-toggle-register');
  if (await reg.isVisible()) await reg.click();
  await page.getByTestId('login-submit').click();
  await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 30000 });
  await page.waitForTimeout(4000);

  console.log('\n===== NON-2XX REQUESTS (deduped) =====');
  const byUrl = new Map();
  for (const r of non2xx) {
    const k = `${r.status} ${r.method} [${r.resourceType}] ${r.url}`;
    byUrl.set(k, (byUrl.get(k) || 0) + 1);
  }
  for (const [k, n] of byUrl) console.log(`  x${n}  ${k}`);
  if (byUrl.size === 0) console.log('  (none)');

  console.log('\n===== CONSOLE ERRORS (deduped) =====');
  const byMsg = new Map();
  for (const m of consoleMsgs) {
    const key = JSON.stringify({ text: m.text, args: m.args });
    byMsg.set(key, (byMsg.get(key) || 0) + 1);
  }
  let idx = 0;
  for (const [k, n] of byMsg) {
    idx++;
    const o = JSON.parse(k);
    console.log(`  [${idx}] x${n}`);
    console.log(`      text: ${o.text.slice(0, 400)}`);
    if (o.args.length) console.log(`      args: ${JSON.stringify(o.args).slice(0, 800)}`);
    if (idx >= 20) { console.log(`  ... (${byMsg.size - idx} more unique)`); break; }
  }
  if (byMsg.size === 0) console.log('  (none)');

  await browser.close();
})().catch(e => { console.error('FATAL', e); process.exit(2); });
