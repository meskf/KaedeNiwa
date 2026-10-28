// test-14: DM wasm 桥（严格版）
// 断言：openDm 成功 → 双方 list 都含 conv → sendDm 成功 → 对端 history 能看到该消息
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }

(async () => {
  console.log('🧪 测试 14: DM wasm 桥（严格）\n');
  await checkPrereqs();
  const browser = await launch();
  const u0Name = randName('u0');
  const u1Name = randName('u1');
  try {
    const u0 = await (await browser.newContext()).newPage();
    const u1 = await (await browser.newContext()).newPage();
    const u0Logs = [], u1Logs = [];
    u0.on('console', m => u0Logs.push(m.text()));
    u1.on('console', m => u1Logs.push(m.text()));

    await login(u0, u0Name); console.log('  u0 =', u0Name);
    await login(u1, u1Name); console.log('  u1 =', u1Name);
    await u0.waitForTimeout(3000);

    await u0.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.dm), null, { timeout: 10000 });
    await u1.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.dm), null, { timeout: 10000 });

    // Step 1: u0 初始 DM 列表
    const initList = await u0.evaluate(() => window.__kaede_debug__.dm.list());
    assert(Array.isArray(initList), 'dm.list 应返回数组');
    console.log('  1. ✅ u0 初始 DM 列表长度 =', initList.length);

    // Step 2: u0 打开与 u1 的 DM
    const conv = await u0.evaluate((peer) => window.__kaede_debug__.dm.open(peer), u1Name);
    assert(conv && conv.conv_id, 'openDm 未返回 conv_id');
    console.log('  2. ✅ conv =', String(conv.conv_id).slice(0, 20), 'peer =', conv.peer_login_id);

    // Step 3: u0 列表中应含该 conv
    const list0 = await u0.evaluate(() => window.__kaede_debug__.dm.list());
    assert(Array.isArray(list0) && list0.some(c => c.conv_id === conv.conv_id), 'u0 DM 列表未含新 conv');
    console.log('  3. ✅ u0 列表含该 conv');

    // Step 4: u1 应能看到该 conv（等待握手下发）
    await u1.waitForTimeout(5000);
    const list1 = await u1.evaluate(() => window.__kaede_debug__.dm.list());
    const u1HasConv = Array.isArray(list1) && list1.some(c => c.peer_login_id === u0Name);
    if (!u1HasConv) {
      u1Logs.filter(l => /dm|mls|error|welcome|handshake|conversation/i.test(l)).slice(-15).forEach(l => console.log('    [u1 log]', l));
      throw new Error('u1 未看到与 u0 的 DM 会话（握手失败）');
    }
    console.log('  4. ✅ u1 侧看到该 conv');

    // Step 5: u0 发 DM（失败即抛）
    const text = 'DM_STRICT_' + Date.now();
    await u0.evaluate(({ id, t }) => window.__kaede_debug__.dm.send(id, t), { id: conv.conv_id, t: text });
    console.log('  5. ✅ u0 sendDm 成功');

    // Step 6: u1 history 应能看到该消息
    await u1.waitForTimeout(6000);
    const u1Conv = (await u1.evaluate(() => window.__kaede_debug__.dm.list())).find(c => c.peer_login_id === u0Name);
    assert(u1Conv, 'u1 找不到与 u0 的 conv（第二次）');
    const u1Hist = await u1.evaluate(({ id }) => window.__kaede_debug__.dm.history(id, 50), { id: u1Conv.conv_id });
    const found = Array.isArray(u1Hist) && u1Hist.some(m => (m.plaintext || '').includes(text));
    if (!found) {
      u1Logs.filter(l => /dm|mls|error|decrypt|application/i.test(l)).slice(-15).forEach(l => console.log('    [u1 log]', l));
      throw new Error(`u1 history 未找到消息 "${text}"（实际 ${Array.isArray(u1Hist) ? u1Hist.length : 'non-array'} 条）`);
    }
    console.log('  6. ✅ u1 history 已收到消息');

    console.log('\n🎉 测试 14 通过（严格判据）');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 14 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
