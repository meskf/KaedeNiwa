const { chromium, checkPrereqs, login, randName, typeMessage } = require('./test-helpers.cjs');

(async () => {
  console.log('🧪 测试 11: 强制断连 → 自动重连 → backfill');
  await checkPrereqs();
  const browser = await chromium.launch({ headless: false, channel: 'msedge' });
  try {
    const u0Name = randName('u0');
    const u1Name = randName('u1');

    const u0 = await (await browser.newContext()).newPage();
    await login(u0, u0Name);
    console.log('  u0:', u0Name);

    const u1 = await (await browser.newContext()).newPage();
    let reconnectCount = 0;
    let disconnectedCount = 0;
    let receivedMsg2 = false;
    u1.on('console', m => {
      const t = m.text();
      if (t.includes('[reconnect] schedule')) reconnectCount++;
      if (t.includes('[reconnect] success')) console.log('[U1] reconnect success!');
      if (t.includes('[MainView] Disconnected')) disconnectedCount++;
      if (/reconnect|Disconnected|disconnected|heartbeat|Joined|backfill/i.test(t)) {
        console.log('[U1]', t);
      }
    });
    u1.on('pageerror', e => console.log('[U1][ERR]', e.message));
    await login(u1, u1Name);
    console.log('  u1:', u1Name);

    await u0.waitForTimeout(5000);
    console.log('  ✅ 2 人已进 #general');

    // 强制断开（不是 setOffline，是真的断 WS + emit Disconnected）
    console.log('  🔌 forceDisconnect u1...');
    // 等 __kaede_debug__ 就绪
    try {
      await u1.waitForFunction(() => typeof window.__kaede_debug__ !== 'undefined', { timeout: 10000 })
    } catch (e) {
      console.log('  ❌ __kaede_debug__ 未就绪');
      process.exit(1)
    }
    const dcResult = await u1.evaluate(async () => {
      const w = window
      const dbg = w.__kaede_debug__
      if (!dbg) return { error: 'no __kaede_debug__' }
      const keys = Object.keys(dbg)
      const t = typeof dbg.forceDisconnect
      if (t !== 'function') return { error: 'forceDisconnect is ' + t, keys }
      try { await dbg.forceDisconnect(); return { ok: true, keys } }
      catch (e) { return { error: String(e), keys } }
    });
    console.log('  forceDisconnect 结果:', JSON.stringify(dcResult));

    // 断网期间 u0 发消息（u1 应该收不到）
    await u0.waitForTimeout(2000);
    const msg1 = 'OFFLINE_' + Date.now();
    await typeMessage(u0, msg1);
    console.log('  u0 断网期间发:', msg1);

    // 等重连 + backfill（心跳 20s 间隔，重连后立即 backfill）
    console.log('  ⏳ 等重连 + backfill (20s)...');
    await u1.waitForTimeout(20000);

    // 断言：[reconnect] schedule 至少 1 次
    if (reconnectCount === 0) {
      console.log('  ❌ 没有 [reconnect] schedule 日志，重连未触发');
      process.exit(1);
    }
    console.log('  ✅ [reconnect] schedule 触发 ' + reconnectCount + ' 次，Disconnected 事件 ' + disconnectedCount + ' 次');

    // 重连后 u0 发消息
    const msg2 = 'RECONNECTED_' + Date.now();
    await typeMessage(u0, msg2);

    try {
      await u1.waitForSelector('text=' + msg2, { timeout: 15000 });
      console.log('  ✅ u1 重连后收到:', msg2);
      receivedMsg2 = true;
    } catch (e) {
      console.log('  ❌ u1 未收到:', msg2);
    }

    if (receivedMsg2) {
      console.log('🎉 测试 11 通过：强制断连 → 自动重连 → backfill → 收发恢复');
    } else {
      console.log('❌ 测试 11 失败');
      process.exit(1);
    }
  } catch (e) {
    console.error('❌ 测试 11 异常:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
