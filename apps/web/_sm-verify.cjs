const { chromium } = require('playwright');
const fs = require('fs');

function posOf(page) {
  return page.evaluate(() => {
    const els = Array.from(document.querySelectorAll('div.fixed')).filter(el => {
      const r = el.getBoundingClientRect();
      return r.width > 30 && r.width < 300 && r.height > 300;
    });
    if (!els.length) return null;
    const r = els[0].getBoundingClientRect();
    return { x: r.x|0, y: r.y|0, w: r.width|0, h: r.height|0, right: r.right|0 };
  });
}

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: false });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  await page.goto('http://localhost:5173');
  await page.waitForTimeout(1200);
  const name = 'u' + Math.floor(Math.random()*1e8);
  await page.locator('[data-testid="login-id"]').fill(name);
  await page.locator('[data-testid="login-password"]').fill('password123');
  await page.locator('[data-testid="login-toggle-register"]').click();
  await page.waitForTimeout(300);
  await page.locator('[data-testid="login-submit"]').click();
  await page.waitForSelector('[data-testid="message-input"]', { timeout: 20000 });
  await page.waitForTimeout(2000);

  // 注入全局 mousemove 记录器
  await page.evaluate(() => {
    window.__mv = [];
    document.addEventListener('mousemove', e => {
      if (e.clientX <= 30) window.__mv.push({ x: e.clientX, y: e.clientY, t: Date.now() });
      if (window.__mv.length > 100) window.__mv.shift();
    }, true);
    window.__enter = [];
    document.querySelectorAll('div.fixed').forEach(el => {
      el.addEventListener('mouseenter', () => window.__enter.push({ ev: 'enter', cls: el.className.slice(0,50) }));
      el.addEventListener('mouseleave', () => window.__enter.push({ ev: 'leave', cls: el.className.slice(0,50) }));
    });
  });

  const before = await posOf(page);
  console.log('before: ' + JSON.stringify(before));

  // 从屏幕中央平滑滑到左边缘（steps=40，模拟真实鼠标）
  await page.mouse.move(700, before.y + before.h / 2, { steps: 1 });
  await page.mouse.move(5, before.y + before.h / 2, { steps: 40 });
  await page.waitForTimeout(1200);
  const after1 = await posOf(page);
  console.log('after smooth-to-left: ' + JSON.stringify(after1));

  // 再往左多推一点（edge 强制）
  await page.mouse.move(0, before.y + before.h / 2, { steps: 5 });
  await page.waitForTimeout(800);
  const after2 = await posOf(page);
  console.log('after edge-0: ' + JSON.stringify(after2));

  // 移开
  await page.mouse.move(800, 400, { steps: 30 });
  await page.waitForTimeout(1000);
  const after3 = await posOf(page);
  console.log('after move-away: ' + JSON.stringify(after3));

  const events = await page.evaluate(() => ({ mv: window.__mv.slice(-20), enter: window.__enter }));
  console.log('mv-at-left-edge (last 20): ' + JSON.stringify(events.mv, null, 2));
  console.log('enter/leave events: ' + JSON.stringify(events.enter, null, 2));

  fs.writeFileSync('_sm-verify.json', JSON.stringify({ before, after1, after2, after3, events }, null, 2));
  await browser.close();
})();
