const { chromium } = require('playwright');
(async () => {
  const stamp = Date.now().toString(36);
  const browser = await chromium.launch({ headless: true });
  const users = [];
  for (let i = 0; i < 2; i++) {
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
    const page = await ctx.newPage();
    const name = `diag${i}-${stamp}`;
    await page.goto('http://127.0.0.1:5173/', { timeout: 30000 });
    await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 20000 });
    await page.getByTestId('login-id').fill(name);
    await page.getByTestId('login-password').fill('password123');
    const reg = page.getByTestId('login-toggle-register');
    if (await reg.isVisible()) await reg.click();
    await page.getByTestId('login-submit').click();
    await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 30000 });
    await page.waitForFunction(() => !!(window).__kaede_debug__?.events, null, { timeout: 20000 });
    users.push({ page, name, ctx });
  }
  console.log('users:', users.map(u => u.name).join(', '));
  await users[0].page.waitForTimeout(3000);

  // u0 发 3 条
  const texts = [];
  for (let i = 0; i < 3; i++) {
    const t = `D${i}_${stamp}`;
    texts.push(t);
    await users[0].page.getByTestId('message-input').click();
    await users[0].page.keyboard.type(t);
    await users[0].page.keyboard.press('Enter');
    await users[0].page.waitForTimeout(300);
  }
  console.log('sent:', texts.join(', '));
  await users[0].page.waitForTimeout(4000);

  for (let i = 0; i < users.length; i++) {
    const dump = await users[i].page.evaluate(async (t) => {
      const dbg = (window).__kaede_debug__;
      const st = dbg.store.getState();
      const ch = st.currentChannel;

      // 事件层
      const events = dbg.events.list();
      const evByType = {};
      for (const e of events) {
        const keys = Object.keys(e);
        for (const k of keys) evByType[k] = (evByType[k] || 0) + 1;
      }

      // 相关事件原始
      const interesting = events.filter((e) => {
        const t = Object.keys(e)[0];
        return t === 'ChannelMessage' || t === 'AppMessageReceived';
      }).map((e) => {
        const t = Object.keys(e)[0];
        const d = e[t] || {};
        let text = '';
        try {
          const pt = d.plaintext || d.payload;
          if (Array.isArray(pt)) text = new TextDecoder().decode(new Uint8Array(pt)).slice(0, 40);
          else if (typeof pt === 'string') text = pt.slice(0, 40);
        } catch {}
        return { type: t, id: d.id, ts: d.ts, from: d.from, text };
      });

      // store 层
      const msgs = (st.messages[ch] || []).map((m) => ({
        id: m.id, ts: m.ts, sender: m.sender, text: (m.plaintext || '').slice(0, 40),
      }));

      // DOM 层
      const rows = Array.from(document.querySelectorAll('[data-testid="message-row"]')).map((el) => ({
        ts: el.getAttribute('data-msg-ts'),
        sender: el.getAttribute('data-msg-sender'),
      }));

      return { ch, evByType, interesting, storeMsgs: msgs, domRows: rows };
    }, texts);

    console.log(`\n===== USER ${i}: ${users[i].name} (ch=${dump.ch}) =====`);
    console.log(`events by type:`, JSON.stringify(dump.evByType));
    console.log(`\n-- interesting events (ChannelMessage / AppMessageReceived) --`);
    dump.interesting.forEach((e, idx) => {
      console.log(`  [${idx}] ${e.type}  id=${e.id}  ts=${e.ts}  from=${e.from}  text=${e.text}`);
    });
    console.log(`\n-- store.messages[${dump.ch}] (${dump.storeMsgs.length}) --`);
    dump.storeMsgs.forEach((m, idx) => {
      console.log(`  [${idx}] id=${m.id} ts=${m.ts} sender=${m.sender} text=${m.text}`);
    });
    const storeSeen = {};
    for (const m of dump.storeMsgs) {
      if (m.text && m.text.startsWith('D')) {
        storeSeen[m.text] = (storeSeen[m.text] || 0) + 1;
      }
    }
    console.log(`-- store counts per sent text:`, JSON.stringify(storeSeen));

    console.log(`\n-- DOM rows (${dump.domRows.length}) --`);
    const domSeen = {};
    for (const r of dump.domRows) {
      const k = `${r.ts}|${r.sender}`;
      domSeen[k] = (domSeen[k] || 0) + 1;
    }
    const dupDom = Object.entries(domSeen).filter(([_, n]) => n > 1);
    console.log(`-- DOM duplicates: ${dupDom.length} unique (ts|sender) has >1 rows`);
    dupDom.slice(0, 10).forEach(([k, n]) => console.log(`  ${k} × ${n}`));
  }
  await browser.close();
})().catch(e => { console.error('FATAL', e); process.exit(2); });
