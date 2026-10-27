const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');
const BASE = 'http://127.0.0.1:5173';
const PASS = 'password123';

(async () => {
  const dir = path.join('../../screenshots', 'diag-' + Date.now());
  fs.mkdirSync(dir, { recursive: true });
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const logs = [];
  page.on('console', m => logs.push('[' + m.type() + '] ' + m.text()));
  page.on('pageerror', e => logs.push('[pageerror] ' + e.message));
  const shot = async n => { await page.screenshot({ path: path.join(dir, n + '.png') }); console.log('SHOT', path.join(dir, n + '.png')); };

  await page.goto(BASE, { waitUntil: 'domcontentloaded' });
  await page.evaluate(() => { localStorage.removeItem('kaede-lang'); localStorage.removeItem('kaede-theme') });
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForSelector('[data-testid="login-id"], [data-testid="message-input"]', { timeout: 25000 });
  if (await page.locator('[data-testid="login-id"]').count() > 0) {
    const name = 'd' + Math.random().toString(36).slice(2, 7);
    await page.fill('[data-testid="login-id"]', name);
    await page.fill('[data-testid="login-password"]', PASS);
    await page.click('[data-testid="login-toggle-register"]');
    await page.click('[data-testid="login-submit"]');
  }
  await page.waitForSelector('[data-testid="message-input"]', { timeout: 20000 });
  await page.waitForTimeout(1500);

  const info = await page.evaluate(() => {
    const footer = document.querySelector('footer');
    const navSpans = footer ? Array.from(footer.querySelectorAll('span.cursor-pointer')).map(s => ({
      text: JSON.stringify(s.textContent),
      classes: s.className.slice(0, 100),
    })) : null;
    const headings = Array.from(document.querySelectorAll('h1,h2,h3')).map(h => h.textContent);
    const hasSettingsView = !!document.querySelector('[data-testid="settings-login-id"]');
    const bodyText = document.body.innerText.slice(0, 500);
    return {
      footerExists: !!footer,
      navSpans,
      headings,
      hasSettingsView,
      bodyText,
      lang: localStorage.getItem('kaede-lang'),
      theme: localStorage.getItem('kaede-theme'),
    };
  });
  console.log('DOM_INFO', JSON.stringify(info, null, 2));
  await shot('01-initial');

  console.log('\n--- footer spans ---');
  const spans = await page.locator('footer span.cursor-pointer').count();
  console.log('total:', spans);
  for (let i = 0; i < spans; i++) {
    const txt = await page.locator('footer span.cursor-pointer').nth(i).textContent();
    console.log(`  [${i}] = ${JSON.stringify(txt)}`);
  }

  console.log('\n--- try click #3 (期望 Settings) ---');
  try {
    await page.locator('footer span.cursor-pointer').nth(3).click();
    await page.waitForTimeout(800);
    const after = await page.evaluate(() => ({
      hasSettingsView: !!document.querySelector('[data-testid="settings-login-id"]'),
      headings: Array.from(document.querySelectorAll('h1,h2,h3')).map(h => h.textContent),
      bodyText: document.body.innerText.slice(0, 400),
    }));
    console.log('AFTER_CLICK', JSON.stringify(after, null, 2));
    await shot('02-after-click-settings');
  } catch (e) { console.log('CLICK_ERR', String(e)); }

  console.log('\n--- console logs (前 30) ---');
  logs.slice(0, 30).forEach(l => console.log(l));
  console.log('TOTAL_LOGS', logs.length);

  await browser.close();
})().catch(e => { console.error('FATAL', e); process.exit(1); });
