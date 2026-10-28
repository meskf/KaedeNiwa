const { chromium } = require('playwright');
const fs = require('fs');
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const failed = [];
  page.on('response', r => { if (r.status() >= 400) failed.push(r.status() + ' ' + r.url()); });
  page.on('console', m => { if (m.type() === 'error') failed.push('[console] ' + m.text().slice(0,200)); });
  await page.goto('http://localhost:5173');
  await page.waitForTimeout(1500);

  const data = await page.evaluate(() => {
    const pick = (el) => {
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      return {
        tag: el.tagName.toLowerCase(),
        type: el.getAttribute('type'),
        placeholder: el.getAttribute('placeholder'),
        name: el.getAttribute('name'),
        text: (el.innerText || '').trim().slice(0, 80),
        ariaLabel: el.getAttribute('aria-label'),
        testid: el.getAttribute('data-testid'),
        visible: r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none',
        rect: [r.x|0, r.y|0, r.width|0, r.height|0],
      };
    };
    return {
      lang: document.documentElement.lang,
      title: document.title,
      bodyText: document.body.innerText.slice(0, 600),
      inputs: Array.from(document.querySelectorAll('input, textarea')).map(pick),
      buttons: Array.from(document.querySelectorAll('button, [role="button"]')).map(pick),
      links: Array.from(document.querySelectorAll('a')).map(pick),
      storage: (() => { const o={}; for (let i=0;i<localStorage.length;i++){const k=localStorage.key(i); o[k]=localStorage.getItem(k).slice(0,80);} return o; })(),
    };
  });
  data.failed = failed.slice(0, 15);
  const out = JSON.stringify(data, null, 2);
  fs.writeFileSync('_login-dump.json', out);
  console.log(out);
  await browser.close();
})();
