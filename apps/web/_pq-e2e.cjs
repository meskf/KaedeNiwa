const { chromium } = require('playwright');
const fs = require('fs');

const BASE = 'http://localhost:5173';
const ts = Date.now();
const aliceName = 'alice-pq-' + Math.floor(Math.random()*1e6);
const bobName = 'bob-pq-' + Math.floor(Math.random()*1e6);

async function register(page, name) {
  await page.goto(BASE);
  await page.waitForTimeout(1200);
  await page.locator('[data-testid="login-id"]').fill(name);
  await page.locator('[data-testid="login-password"]').fill('password123');
  await page.locator('[data-testid="login-toggle-register"]').click();
  await page.waitForTimeout(300);
  await page.locator('[data-testid="login-submit"]').click();
  await page.waitForSelector('[data-testid="message-input"]', { timeout: 25000 });
}

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const aCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const bCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const a = await aCtx.newPage();
  const b = await bCtx.newPage();

  const errs = [];
  [['alice', a], ['bob', b]].forEach(([n, p]) => {
    p.on('pageerror', e => errs.push(`[${n}] pageerror: ${String(e).slice(0,200)}`));
    p.on('console', m => { if (m.type() === 'error') errs.push(`[${n}] console: ${m.text().slice(0,200)}`); });
  });

  console.log('=== 1. Alice 注册 (' + aliceName + ') ===');
  await register(a, aliceName);
  console.log('  ✅ alice logged in');
  await a.waitForTimeout(2000);

  console.log('=== 2. Bob 注册 (' + bobName + ') ===');
  await register(b, bobName);
  console.log('  ✅ bob logged in');
  await b.waitForTimeout(2500);

  // 找当前频道（从 DOM 或 localStorage）
  const chId = await a.evaluate(() => {
    // 尝试从任一频道按钮拿 data-channel-id
    const el = document.querySelector('[data-channel-id]');
    if (el) return el.getAttribute('data-channel-id');
    // fallback: 用 localStorage 里可能的记录
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k && k.includes('channel')) {
        const v = localStorage.getItem(k);
        if (v && v.length < 100) return v;
      }
    }
    return 'general';
  });
  console.log('  channel =', chId);

  // 双方加入频道（点击第一个频道保证同频道）
  const firstChSelectors = [
    '[data-testid^="channel-"]',
    'button:has-text("general")',
    'a:has-text("general")',
    '[role="button"]:has-text("general")',
  ];
  for (const sel of firstChSelectors) {
    const loc = a.locator(sel).first();
    if (await loc.isVisible().catch(()=>false)) {
      await loc.click().catch(()=>{});
      break;
    }
  }
  await a.waitForTimeout(800);
  for (const sel of firstChSelectors) {
    const loc = b.locator(sel).first();
    if (await loc.isVisible().catch(()=>false)) {
      await loc.click().catch(()=>{});
      break;
    }
  }
  await b.waitForTimeout(1500);

  // 双方 channelState
  const aState = await a.evaluate(async (ch) => await window.__kaede_debug__.channelState(ch).catch(e => ({ error: String(e) })), chId);
  const bState = await b.evaluate(async (ch) => await window.__kaede_debug__.channelState(ch).catch(e => ({ error: String(e) })), chId);
  console.log('  alice state:', JSON.stringify(aState));
  console.log('  bob   state:', JSON.stringify(bState));

  // 发消息
  console.log('=== 3. Alice 发 PQ 加密消息 ===');
  const msg = 'PQ-hello-' + ts;
  await a.fill('[data-testid="message-input"]', msg);
  await a.press('[data-testid="message-input"]', 'Enter');
  await a.waitForTimeout(1000);

  // 断言 Bob 收到
  console.log('=== 4. 断言 Bob 收到解密后的消息 ===');
  let passed = false;
  try {
    await b.waitForSelector(`text=${msg}`, { timeout: 10000 });
    passed = true;
    console.log('  ✅ PASS: Bob 收到并解密了 PQ 加密消息');
  } catch {
    console.log('  ❌ FAIL: Bob 未收到消息');
  }

  // Bob 回一条，Alice 收
  if (passed) {
    console.log('=== 5. Bob 回消息 ===');
    const reply = 'PQ-reply-' + ts;
    await b.fill('[data-testid="message-input"]', reply);
    await b.press('[data-testid="message-input"]', 'Enter');
    try {
      await a.waitForSelector(`text=${reply}`, { timeout: 10000 });
      console.log('  ✅ PASS: Alice 收到 Bob 回复');
    } catch {
      console.log('  ❌ FAIL: Alice 未收到 Bob 回复');
      passed = false;
    }
  }

  // 报告
  const report = [];
  report.push('=== PQ E2E REPORT ===');
  report.push('alice=' + aliceName + ' bob=' + bobName + ' channel=' + chId);
  report.push('alice state: ' + JSON.stringify(aState));
  report.push('bob state:   ' + JSON.stringify(bState));
  report.push('keys equal: ' + (aState.key_b64 && aState.key_b64 === bState.key_b64));
  report.push('epoch equal: ' + (aState.epoch === bState.epoch));
  report.push('PASS: ' + passed);
  report.push('errors (' + errs.length + '):');
  errs.slice(0, 15).forEach(e => report.push('  ' + e));
  const reportPath = '_pq-e2e-report-' + ts + '.txt';
  fs.writeFileSync(reportPath, report.join('\n'));
  console.log('\n' + report.join('\n'));
  console.log('\n[report] ' + reportPath);

  await browser.close();
  process.exit(passed ? 0 : 1);
})();
