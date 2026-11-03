const { chromium } = require('playwright');

// 测试用：直接绕过限流——注册只跑一次，然后复用 localStorage token（不注册新号）
// 但为了简单，这里用 type + 单次注册，跑 2 个用户（避免撞 10 次上限）
(async () => {
  const stamp = Date.now().toString(36);
  const browser = await chromium.launch({ headless: true });
  const users = [];
  for (let i = 0; i < 2; i++) {
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
    const page = await ctx.newPage();
    const name = `d${i}-${stamp}`;
    await page.goto('http://127.0.0.1:5173/', { timeout: 30000 });

    const idBox = page.getByTestId('login-id');
    await idBox.waitFor({ state: 'visible', timeout: 20000 });
    await idBox.click();
    await page.keyboard.type(name, { delay: 15 });

    const pwBox = page.getByTestId('login-password');
    await pwBox.click();
    await page.keyboard.type('password123', { delay: 15 });

    const reg = page.getByTestId('login-toggle-register');
    if (await reg.isVisible()) await reg.click();

    await page.getByTestId('login-submit').click();

    // 观察是否 5s 内进入主界面，否则 dump 错误
    try {
      await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 15000 });
    } catch {
      const err = await page.locator('body').innerText();
      console.error(`\n[user ${i}] login 失败。页面文本:\n${err.slice(0, 400)}`);
      throw new Error(`login timeout for ${name}`);
    }
    await page.waitForFunction(() => !!(window).__kaede_debug__, null, { timeout: 20000 });
    users.push({ page, name, ctx });
    console.log(`[user ${i}] ${name} OK`);
  }

  await users[0].page.waitForTimeout(3000);

  const texts = [];
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 2; i++) {
      const t = `X${i}R${round}_${stamp}`;
      texts.push(t);
      const box = users[i].page.getByTestId('message-input');
      await box.click();
      await users[i].page.keyboard.type(t, { delay: 15 });
      await users[i].page.keyboard.press('Enter');
      await users[i].page.waitForTimeout(250);
    }
  }
  console.log(`\nsent 6 messages (typed): ${texts.join(', ')}`);
  await users[0].page.waitForTimeout(4000);

  for (let i = 0; i < users.length; i++) {
    const dump = await users[i].page.evaluate((texts) => {
      const dbg = (window).__kaede_debug__;
      const st = dbg.store.getState();
      const ch = st.currentChannel;
      const msgs = st.messages[ch] || [];
      const counts = {};
      for (const t of texts) {
        counts[t] = msgs.filter(m => m.plaintext === t).map(m => ({ id: m.id, ts: m.ts, sender: m.sender }));
      }
      const events = dbg.events.list();
      const relevant = events.filter((e) => {
        const t = Object.keys(e)[0];
        if (t !== 'ChannelMessage' && t !== 'AppMessageReceived') return false;
        const d = e[t] || {};
        let txt = '';
        try {
          const pt = d.plaintext || d.payload;
          if (Array.isArray(pt)) txt = new TextDecoder().decode(new Uint8Array(pt));
          else if (typeof pt === 'string') txt = pt;
        } catch {}
        return texts.some(tx => txt.includes(tx));
      }).map((e) => {
        const t = Object.keys(e)[0];
        const d = e[t] || {};
        let text = '';
        try {
          const pt = d.plaintext || d.payload;
          if (Array.isArray(pt)) text = new TextDecoder().decode(new Uint8Array(pt));
          else if (typeof pt === 'string') text = pt;
        } catch {}
        const tag = texts.find(tx => text.includes(tx)) || '?';
        return { type: t, id: d.id, ts: d.ts, from: d.from, tag };
      });
      return { ch, counts, relevant, totalMsgs: msgs.length };
    }, texts);

    console.log(`\n===== USER ${i}: ${users[i].name} (${dump.totalMsgs} msgs store) =====`);
    const dups = Object.entries(dump.counts).filter(([_, arr]) => arr.length > 1);
    console.log(`duplicates: ${dups.length}`);
    for (const [t, arr] of dups) {
      console.log(`  [DUP] ${t} ×${arr.length}`);
      arr.forEach((x, k) => console.log(`    [${k}] id=${x.id} ts=${x.ts} sender=${x.sender}`));
    }
    console.log(`events matching sent: ${dump.relevant.length}`);
    const evPerTag = {};
    for (const e of dump.relevant) {
      if (!evPerTag[e.tag]) evPerTag[e.tag] = [];
      evPerTag[e.tag].push(`${e.type}(id=${e.id},ts=${e.ts})`);
    }
    for (const [tag, evs] of Object.entries(evPerTag)) {
      console.log(`  ${tag}: ${evs.join(' + ')}`);
    }
  }
  await browser.close();
})().catch(e => { console.error('FATAL', e.message); process.exit(2); });
