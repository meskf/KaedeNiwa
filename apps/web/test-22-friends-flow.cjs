// test-22: 好友完整流程（wasm 直调）
// 覆盖：初始空 → sendRequest → outgoing/incoming → accept → friend → openDm → sendDm → remove
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) {
  if (!cond) throw new Error('断言失败: ' + msg);
}

(async () => {
  console.log('🧪 测试 22: 好友流程（sendRequest/accept/remove + DM 打通）\n');
  await checkPrereqs();
  const browser = await launch();
  const u0Name = randName('f0');
  const u1Name = randName('f1');
  try {
    const u0 = await (await browser.newContext()).newPage();
    const u1 = await (await browser.newContext()).newPage();
    const u0Logs = [], u1Logs = [];
    u0.on('console', m => u0Logs.push(m.text()));
    u1.on('console', m => u1Logs.push(m.text()));

    await login(u0, u0Name); console.log('  u0 =', u0Name);
    await login(u1, u1Name); console.log('  u1 =', u1Name);
    await u0.waitForTimeout(2000);

    // 等 debug 就绪
    await u0.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.friends), null, { timeout: 10000 });
    await u1.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.friends), null, { timeout: 10000 });
    console.log('  __kaede_debug__.friends 就绪');

    // Step 1: u0 初始好友列表为空
    const initF = await u0.evaluate(() => window.__kaede_debug__.friends.list());
    assert(Array.isArray(initF), 'friends.list 应返回数组');
    assert(initF.length === 0, `u0 初始好友列表应为空，实际 ${initF.length}`);
    console.log('  1. ✅ 初始好友列表为空');

    // Step 2: u0 → u1 发好友请求
    await u0.evaluate((peer) => window.__kaede_debug__.friends.sendRequest(peer), u1Name);
    console.log('  2. ✅ u0 → u1 请求已发送');
    await u0.waitForTimeout(800);

    // Step 3: u0 侧应为 outgoing
    const u0Out = await u0.evaluate(() => window.__kaede_debug__.friends.list());
    const out = u0Out.find(f => f.login_id === u1Name);
    assert(out, 'u0 好友列表应包含 u1');
    assert(out.direction === 'outgoing', `u0 direction 应为 outgoing，实际 ${out.direction}`);
    console.log('  3. ✅ u0 侧 = outgoing');

    // Step 4: u1 侧应为 incoming
    await u1.waitForTimeout(2000);
    const u1Inc = await u1.evaluate(() => window.__kaede_debug__.friends.list());
    const inc = u1Inc.find(f => f.login_id === u0Name);
    assert(inc, 'u1 好友列表应包含 u0');
    assert(inc.direction === 'incoming', `u1 direction 应为 incoming，实际 ${inc.direction}`);
    console.log('  4. ✅ u1 侧 = incoming');

    // Step 5: u1 接受
    await u1.evaluate((peer) => window.__kaede_debug__.friends.accept(peer), u0Name);
    console.log('  5. ✅ u1 已接受');
    await u0.waitForTimeout(2000);

    // Step 6: 双方 direction = friend
    const u0Fin = await u0.evaluate(() => window.__kaede_debug__.friends.list());
    const u1Fin = await u1.evaluate(() => window.__kaede_debug__.friends.list());
    const u0f = u0Fin.find(f => f.login_id === u1Name);
    const u1f = u1Fin.find(f => f.login_id === u0Name);
    assert(u0f && u0f.direction === 'friend', `u0 应为 friend，实际 ${u0f && u0f.direction}`);
    assert(u1f && u1f.direction === 'friend', `u1 应为 friend，实际 ${u1f && u1f.direction}`);
    console.log('  6. ✅ 双方 = friend');

    // Step 7: u0 打开 DM
    const conv = await u0.evaluate((peer) => window.__kaede_debug__.dm.open(peer), u1Name);
    assert(conv && conv.conv_id, 'openDm 未返回 conv_id');
    console.log('  7. ✅ DM conv =', String(conv.conv_id).slice(0, 12));

    // Step 8: u0 发 DM
    const text = 'FRIEND_FLOW_' + Date.now();
    await u0.evaluate(({ id, t }) => window.__kaede_debug__.dm.send(id, t), { id: conv.conv_id, t: text });
    console.log('  8. ✅ DM 已发送');
    await u1.waitForTimeout(5000);

    // Step 9: u1 应看到会话
    const u1Convs = await u1.evaluate(() => window.__kaede_debug__.dm.list());
    const u1HasConv = Array.isArray(u1Convs) && u1Convs.some(c => c.peer_login_id === u0Name);
    if (!u1HasConv) {
      u1Logs.filter(l => /dm|mls|error|welcome|handshake/i.test(l)).slice(-12).forEach(l => console.log('    [u1 log]', l));
      throw new Error('u1 未能看到与 u0 的 DM 会话（2 人 DM MLS handshake 可能未完成）');
    }
    console.log('  9. ✅ u1 看到 DM 会话');

    // Step 10: u0 移除好友
    await u0.evaluate((peer) => window.__kaede_debug__.friends.remove(peer), u1Name);
    await u0.waitForTimeout(800);
    const afterRm = await u0.evaluate(() => window.__kaede_debug__.friends.list());
    assert(!afterRm.some(f => f.login_id === u1Name), '移除后 u0 侧不应有 u1');
    console.log('  10. ✅ 好友已移除');

    console.log('\n🎉 测试 22 通过');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 22 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
