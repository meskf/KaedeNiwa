// chaos-2-disconnect.cjs — 断连风暴
//
// 场景：8 用户加入 general，然后：
//   - 每 2-5s 随机断开一个用户 1-3s
//   - 断开期间其他用户照常发消息
//   - 重连后断言：断连用户能看到断连期间所有消息
//
// 不变量：
//   - 8/8 注册 + 加入 general
//   - 消息发送数 = 消息接收数（每个用户）
//   - 所有用户 epoch 最终一致
//   - 0 JS 真错误、0 HTTP 5xx
//
// 运行: node e2e/chaos-2-disconnect.cjs

const { chromium } = require('playwright');
const fs = require('fs');

const BASE = 'http://localhost:5173';
const N_USERS = 8;
const RANDOM = Math.floor(Math.random() * 1e9);
const CYCLES = 5;              // 5 轮断连
const MSG_INTERVAL_MS = 400;   // 每 400ms 发一条

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

async function registerUser(browser, name, label, errs, netErrs) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await ctx.newPage();
  page.on('console', m => {
    if (m.type() !== 'error') return;
    const t = m.text();
    if (t.includes('Failed to load resource')) return;
    if (t.includes('favicon')) return;
    if (t.includes('StrictMode')) return;
    if (t.includes('React DevTools')) return;
    if (t.includes('join timeout') && t.includes('cannot self-create')) return;
    // 断连测试的必然产物：forceDisconnect 强关 WS 时正好有 send 在路上
    // 真实场景下网络断了也会一样。消息本身会走 pending 队列 + 重连后 history 补齐
    if (t.includes('WebSocket is already in CLOSING or CLOSED state')) return;
    errs.push(`[${label}] ${t.slice(0, 200)}`);
  });
  page.on('pageerror', e => {
    const s = String(e);
    if (s.includes('Failed to load resource')) return;
    errs.push(`[${label} pageerror] ${s.slice(0, 200)}`);
  });
  page.on('response', r => {
    if (r.status() >= 500) netErrs.push(`[${label}] ${r.status()} ${r.request().method()} ${r.url()}`);
  });
  await page.goto(BASE);
  await page.waitForTimeout(500);
  await page.locator('[data-testid="login-id"]').fill(name);
  await page.locator('[data-testid="login-password"]').fill('password123');
  await page.locator('[data-testid="login-toggle-register"]').click();
  await page.waitForTimeout(200);
  await page.locator('[data-testid="login-submit"]').click();
  await page.waitForSelector('[data-testid="message-input"]', { timeout: 30000 });
  await page.waitForTimeout(2000);
  return page;
}

(async () => {
  const start = Date.now();
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const errs = [];
  const netErrs = [];
  const users = []; // { page, name, label }

  console.log(`[chaos-2] Stage 1: ${N_USERS} users registering...`);
  for (let i = 0; i < N_USERS; i++) {
    const name = `chaos2-${RANDOM}-${i}`;
    const label = `U${i}`;
    try {
      const page = await registerUser(browser, name, label, errs, netErrs);
      users.push({ page, name, label });
      process.stdout.write('.');
    } catch (e) {
      console.log(`\n  ${label} failed: ${String(e).slice(0, 150)}`);
    }
  }
  console.log(`\n  registered: ${users.length}/${N_USERS}`);
  if (users.length < N_USERS) {
    console.log('  aborting: not all users registered');
    await browser.close();
    process.exit(1);
  }

  console.log(`\n[chaos-2] Stage 2: wait 3s for group convergence...`);
  await sleep(3000);

  // 每用户在 group 里都有编号 0..N-1
  const allMessages = [];      // 所有消息文本
  let msgCounter = 0;
  const nextMsg = (uid) => `chaos2-${RANDOM}-u${uid}-m${msgCounter++}`;

  console.log(`\n[chaos-2] Stage 3: ${CYCLES} rounds of disconnect chaos...`);

  // 后台持续发消息
  let sending = true;
  const senderPromises = users.map(async (u, idx) => {
    while (sending) {
      try {
        const text = nextMsg(idx);
        allMessages.push({ text, from: idx });
        await u.page.locator('[data-testid="message-input"]').fill(text);
        await u.page.locator('[data-testid="message-input"]').press('Enter');
      } catch (e) {
        // 用户可能正在断连，消息发不出去 —— 记录但不计入"应该送达"列表？
        // 决策：发不出去的算"未发送"，从 allMessages 移除
        const last = allMessages[allMessages.length - 1];
        if (last && last.from === idx) allMessages.pop();
      }
      await sleep(MSG_INTERVAL_MS + Math.random() * 200);
    }
  });

  // 每轮随机断一个用户
  for (let cycle = 0; cycle < CYCLES; cycle++) {
    const victim = users[cycle % users.length];
    const downMs = 1000 + Math.floor(Math.random() * 2000);
    console.log(`  cycle ${cycle + 1}: disconnecting ${victim.label} for ${downMs}ms`);

    try {
      await victim.page.evaluate(() => window.__kaede_debug__.forceDisconnect());
    } catch (e) {
      console.log(`    forceDisconnect failed: ${String(e).slice(0, 120)}`);
    }
    await sleep(downMs);
    // 不需要显式 reconnect —— scheduleReconnect 会自动重连
    // 给重连一点时间
    await sleep(2500);
  }

  // 停发，等最后一次分发
  sending = false;
  await Promise.all(senderPromises);
  console.log(`  sent ${allMessages.length} messages total`);
  console.log(`\n[chaos-2] Stage 4: wait 8s for final fan-out...`);
  await sleep(8000);

  // 断言：每个用户看到所有消息
  console.log(`\n[chaos-2] Stage 5: verifying delivery...`);
  const perUserMissing = [];
  for (let i = 0; i < users.length; i++) {
    const u = users[i];
    const text = await u.page.evaluate(() => document.body.innerText);
    const missing = allMessages.filter(m => !text.includes(m.text));
    perUserMissing.push({ user: u.label, missing: missing.length, samples: missing.slice(0, 3).map(m => m.text) });
  }
  const usersWithGaps = perUserMissing.filter(x => x.missing > 0);
  const totalMissing = perUserMissing.reduce((s, x) => s + x.missing, 0);
  const totalDeliveries = allMessages.length * users.length;
  const totalDelivered = totalDeliveries - totalMissing;

  // 断言：所有用户 epoch 一致
  const states = await Promise.all(users.map(u =>
    u.page.evaluate(async () => {
      try { return await window.__kaede_debug__.channelState('general'); }
      catch (e) { return { error: String(e) }; }
    })
  ));
  const epochs = states.map(s => s.epoch).filter(e => e !== undefined && e !== null);
  const uniqueEpochs = Array.from(new Set(epochs));
  const allSameEpoch = uniqueEpochs.length === 1 && uniqueEpochs[0] !== undefined;

  // 收尾
  const totalDur = (Date.now() - start) / 1000;
  const pass =
    allSameEpoch &&
    usersWithGaps.length === 0 &&
    netErrs.length === 0 &&
    errs.length === 0;

  console.log(`\n═══ CHAOS-2 REPORT ═══`);
  console.log(`  users:                ${users.length}/${N_USERS}`);
  console.log(`  disconnect cycles:    ${CYCLES}`);
  console.log(`  messages sent:        ${allMessages.length}`);
  console.log(`  messages delivered:   ${totalDelivered}/${totalDeliveries}`);
  console.log(`  users with gaps:      ${usersWithGaps.length}/${users.length}`);
  console.log(`  epoch converged:      ${allSameEpoch} (${uniqueEpochs.join(',')})`);
  console.log(`  total duration:       ${totalDur.toFixed(1)}s`);
  console.log(`  JS errors (real):     ${errs.length}`);
  console.log(`  5xx responses:        ${netErrs.length}`);
  if (errs.length > 0) {
    console.log(`\n  JS errors:`);
    errs.slice(0, 8).forEach(e => console.log(`    ${e}`));
  }
  if (netErrs.length > 0) {
    console.log(`\n  5xx responses:`);
    netErrs.slice(0, 5).forEach(e => console.log(`    ${e}`));
  }
  if (usersWithGaps.length > 0) {
    console.log(`\n  users with missing msgs:`);
    usersWithGaps.slice(0, 5).forEach(r => console.log(`    ${r.user}: ${r.missing} missing, e.g. ${r.samples.join(', ')}`));
  }
  console.log(`\n  PASS: ${pass}`);

  // 落盘
  const report = {
    ts: Date.now(),
    users: users.length,
    disconnect_cycles: CYCLES,
    messages_sent: allMessages.length,
    messages_delivered: totalDelivered,
    messages_total: totalDeliveries,
    users_with_gaps: usersWithGaps.length,
    epoch_converged: allSameEpoch,
    unique_epochs: uniqueEpochs,
    js_errors: errs.length,
    http_5xx: netErrs.length,
    pass,
  };
  fs.writeFileSync(`chaos-2-report-${report.ts}.json`, JSON.stringify(report, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})();
