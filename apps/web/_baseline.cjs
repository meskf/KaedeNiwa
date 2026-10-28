const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const BASE = 'http://localhost:5173';
const TS = Date.now();
const outDir = path.join(__dirname, '..', '..', 'screenshots', 'baseline-' + TS);
fs.mkdirSync(outDir, { recursive: true });

async function domAudit(page) {
  return await page.evaluate(() => {
    const out = [];
    const vw = window.innerWidth, vh = window.innerHeight;
    const h = document.documentElement;
    if (h.scrollWidth > vw + 1) out.push('H-SCROLL doc ' + h.scrollWidth + '>' + vw);
    if (h.scrollHeight > vh + 1) out.push('V-SCROLL doc ' + h.scrollHeight + '>' + vh);
    document.querySelectorAll('body *').forEach(el => {
      const r = el.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) return;
      const cs = getComputedStyle(el);
      if (cs.visibility === 'hidden' || cs.display === 'none') return;
      const cls = (typeof el.className === 'string' ? el.className : '').slice(0, 40);
      const tid = el.getAttribute && el.getAttribute('data-testid');
      const label = el.tagName.toLowerCase() + (tid ? '[tid=' + tid + ']' : (el.id ? '#'+el.id : '') + '.' + cls);
      if (r.right > vw + 2) out.push('OOB-R ' + label + ' right=' + (r.right|0) + ' vw=' + vw);
      if (r.left < -2) out.push('OOB-L ' + label + ' left=' + (r.left|0));
      if (r.bottom > vh + 2) out.push('OOB-B ' + label + ' bottom=' + (r.bottom|0) + ' vh=' + vh);
    });
    return Array.from(new Set(out)).slice(0, 40);
  });
}

async function clickIfVisible(loc, timeout=800) {
  try { if (await loc.isVisible({timeout})) { await loc.click(); return true; } } catch {}
  return false;
}

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const consoleErrs = [];
  const failedReqs = [];
  page.on('console', m => { if (m.type() === 'error') consoleErrs.push(m.text().slice(0, 240)); });
  page.on('pageerror', e => consoleErrs.push('[pageerror] ' + String(e).slice(0, 240)));
  page.on('response', r => { if (r.status() >= 400) failedReqs.push(r.status() + ' ' + r.url()); });

  const audits = {};
  const notes = [];
  async function snap(name) {
    await page.waitForTimeout(400);
    await page.screenshot({ path: path.join(outDir, name + '.png') });
    audits[name] = await domAudit(page);
    console.log('  [snap] ' + name + ' (' + audits[name].length + ' issues)');
  }

  try {
    await page.goto(BASE);
    await page.waitForTimeout(1200);
    await snap('01-login');

    const user = 'alice-' + Math.floor(Math.random()*100000);
    notes.push('user = ' + user);
    await page.locator('[data-testid="login-id"]').fill(user);
    await page.locator('[data-testid="login-password"]').fill('password123');

    const toggled = await clickIfVisible(page.locator('[data-testid="login-toggle-register"]'));
    notes.push('toggled to register = ' + toggled);
    await page.waitForTimeout(500);
    const regBtnText = await page.locator('[data-testid="login-submit"]').innerText().catch(()=>'?');
    notes.push('submit-btn text after toggle = ' + JSON.stringify(regBtnText));
    await page.screenshot({ path: path.join(outDir, '01b-register-mode.png') });

    await page.locator('[data-testid="login-submit"]').click();

    let entered = false;
    try {
      await page.waitForSelector('[data-testid="message-input"]', { timeout: 20000 });
      entered = true;
    } catch {
      notes.push('FAILED to enter main after submit; dumping DOM');
      const dump = await page.evaluate(() => ({
        url: location.href,
        bodyText: document.body.innerText.slice(0, 600),
        testids: Array.from(document.querySelectorAll('[data-testid]')).map(e => e.getAttribute('data-testid')),
      }));
      fs.writeFileSync(path.join(outDir, 'post-submit.json'), JSON.stringify(dump, null, 2));
      await page.screenshot({ path: path.join(outDir, '99-post-submit.png') });
    }

    if (entered) {
      await page.waitForTimeout(2000);
      await snap('02-main-dark');

      await page.fill('[data-testid="message-input"]', 'baseline hello');
      await page.press('[data-testid="message-input"]', 'Enter');
      await page.waitForTimeout(800);
      await snap('03-main-with-msg');

      await page.keyboard.press('Control+K');
      await page.waitForTimeout(500);
      await snap('04-search-palette');
      await page.keyboard.press('Escape').catch(()=>{});

      await clickIfVisible(page.locator('[data-testid="open-settings"]'));
      await page.waitForTimeout(500);
      if (await page.locator('[data-testid="open-settings"]').isVisible().catch(()=>false)) {
        // settings modal didn't open via testid — try fallback list
        await clickIfVisible(page.locator('button[aria-label="设置"], button:has-text("设置")'));
        await page.waitForTimeout(400);
      }
      await snap('05-settings');
      await page.keyboard.press('Escape').catch(()=>{});

      await clickIfVisible(page.locator('[data-testid="nav-friends"]'));
      await page.waitForTimeout(500);
      await snap('06-friends');

      await clickIfVisible(page.locator('[data-testid="nav-dms"]'));
      await page.waitForTimeout(500);
      await snap('07-dms');

      await page.setViewportSize({ width: 1280, height: 800 });
      await page.waitForTimeout(500);
      await snap('08-laptop-1280');

      await page.setViewportSize({ width: 375, height: 667 });
      await page.waitForTimeout(500);
      await snap('09-mobile-375');
    }
  } catch (e) {
    notes.push('EXCEPTION: ' + String(e).slice(0, 300));
    await page.screenshot({ path: path.join(outDir, '99-crash.png') }).catch(()=>{});
  }

  await browser.close();

  const report = [];
  report.push('baseline ' + new Date(TS).toISOString());
  report.push('screens: ' + Object.keys(audits).length + ', failed requests: ' + failedReqs.length + ', console errors: ' + consoleErrs.length);
  report.push('');
  if (notes.length) { report.push('== notes =='); notes.forEach(n => report.push('  ' + n)); report.push(''); }
  for (const [k,v] of Object.entries(audits)) {
    report.push('== ' + k + ' ==');
    if (v.length === 0) report.push('  (clean)');
    else v.forEach(x => report.push('  ' + x));
    report.push('');
  }
  if (failedReqs.length) { report.push('== failed requests =='); failedReqs.slice(0,25).forEach(x => report.push('  ' + x)); }
  if (consoleErrs.length) { report.push('== console errors =='); consoleErrs.slice(0,25).forEach(x => report.push('  ' + x)); }
  const reportPath = path.join(outDir, 'report.txt');
  fs.writeFileSync(reportPath, report.join('\n'));
  console.log('\n======== REPORT ========');
  console.log(report.join('\n'));
  console.log('========================');
  console.log('[baseline] report = ' + reportPath);
})();
