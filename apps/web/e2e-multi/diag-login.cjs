const { chromium } = require('playwright');
(async () => {
  const name = `multi0-${Date.now().toString(36)}-${Math.floor(Math.random()*1e6)}`;
  console.log('[diag] name:', name, 'len:', name.length);

  const browser = await chromium.launch({ headless: false });
  const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
  const page = await ctx.newPage();

  page.on('console', m => {
    if (m.type() === 'error' || m.type() === 'warning') console.log(`[console.${m.type()}]`, m.text().slice(0, 500));
  });
  page.on('pageerror', e => console.log('[pageerror]', e.message));
  page.on('requestfailed', r => {
    const u = r.url();
    if (u.startsWith('data:') || u.includes('favicon')) return;
    console.log('[reqfail]', r.method(), u, '->', r.failure()?.errorText);
  });
  page.on('response', async r => {
    const u = r.url();
    if (u.includes('/api/')) {
      let body = '';
      try { body = (await r.text()).slice(0, 400); } catch {}
      console.log(`[api] ${r.status()} ${r.request().method()} ${u}${body ? ' body=' + body : ''}`);
    }
  });

  console.log('[diag] goto...');
  await page.goto('http://127.0.0.1:5173/', { waitUntil: 'domcontentloaded', timeout: 30000 });
  console.log('[diag] goto done, url=', page.url());

  try {
    await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 15000 });
    console.log('[diag] login-id visible');
  } catch (e) {
    console.log('[diag] login-id NOT visible:', e.message);
    console.log('[diag] body:', await page.evaluate(() => document.body.innerText.slice(0, 800)));
    await page.screenshot({ path: 'e2e-multi/diag-1-no-login.png' });
    await browser.close();
    return;
  }

  await page.getByTestId('login-id').fill(name);
  await page.getByTestId('login-password').fill('password123');
  const reg = page.getByTestId('login-toggle-register');
  if (await reg.isVisible()) { console.log('[diag] toggle register'); await reg.click(); }
  console.log('[diag] click submit');
  await page.getByTestId('login-submit').click();

  await page.waitForTimeout(7000);
  console.log('[diag] after 7s: url=', page.url());
  const body = await page.evaluate(() => document.body.innerText.slice(0, 1500));
  console.log('[diag] body:', body);

  const hasInput = await page.getByTestId('message-input').isVisible().catch(() => false);
  console.log('[diag] message-input visible:', hasInput);
  const hasDebug = await page.evaluate(() => !!(window).__kaede_debug__);
  console.log('[diag] __kaede_debug__ present:', hasDebug);

  await page.screenshot({ path: 'e2e-multi/diag-2-after-submit.png', fullPage: true });
  await browser.close();
  console.log('[diag] DONE');
})().catch(e => { console.error('[diag] FATAL', e); process.exit(2); });
