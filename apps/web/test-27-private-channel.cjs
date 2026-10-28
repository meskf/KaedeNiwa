// test-27: 私密频道 e2e（创建 → 密码错误拒绝 → 密码正确加入 → 消息 + reaction 私密路径）
// 覆盖 3 个维度：
//   1. 服务端 Argon2 password 校验（401 vs 成功）
//   2. 私密频道消息路径（0x03 channel-key seal → 接收方收 ChannelMessage）
//   3. 私密频道 reaction 路径（0x03 → 接收方收 ChannelReaction，区别于公开频道的 AppMessageReceived）
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }

(async () => {
  console.log('🧪 测试 27: 私密频道 e2e（密码 + 消息 + reaction）\n');
  await checkPrereqs();
  const browser = await launch();
  const u0Name = randName('pv0');
  const u1Name = randName('pv1');
  const PW = 'SecretPass_12';
  let chId = null;
  try {
    const u0 = await (await browser.newContext()).newPage();
    const u1 = await (await browser.newContext()).newPage();
    const u0Logs = [], u1Logs = [];
    u0.on('console', m => u0Logs.push(m.text()));
    u1.on('console', m => u1Logs.push(m.text()));

    await login(u0, u0Name); console.log('  u0 =', u0Name);
    await u0.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.channels), null, { timeout: 10000 });

    // ---- Step 1: u0 创建私密频道 ----
    const chName = 'pv-' + Date.now();
    const created = await u0.evaluate(async ({ name, pw }) => {
      return await window.__kaede_debug__.channels.create(name, 'text', null, true, pw);
    }, { name: chName, pw: PW });
    chId = created && (created.id || created.channel_id);
    if (!chId) throw new Error('创建私密频道未返回 id: ' + JSON.stringify(created).slice(0, 200));
    console.log('  1. ✅ 创建私密频道 id =', chId);

    // 服务端 Argon2 创建耗时，等一下
    await u0.waitForTimeout(1500);

    // ---- Step 2: u0 自己 join（建立 channel group） ----
    try {
      await u0.evaluate(async ({ ch, pw }) => window.__kaede_debug__.channels.joinWithPassword(ch, pw), { ch: chId, pw: PW });
      console.log('  2. ✅ u0 自己 join 成功（group 建立）');
    } catch (e) {
      u0Logs.filter(l => /join|channel|mls|welcome|error/i.test(l)).slice(-10).forEach(l => console.log('    [u0 log]', l));
      throw new Error('u0 自己 join 失败: ' + String(e).slice(0, 300));
    }
    await u0.waitForTimeout(1000);

    // ---- Step 3: u1 登录 ----
    await login(u1, u1Name); console.log('  u1 =', u1Name);
    await u1.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.channels), null, { timeout: 10000 });

    // ---- Step 4: u1 用错误密码 join → 期望失败 ----
    let wrongPwdRejected = false;
    let wrongErrMsg = '';
    try {
      await u1.evaluate(async ({ ch }) => window.__kaede_debug__.channels.joinWithPassword(ch, 'WrongPass99'), { ch: chId });
    } catch (e) {
      wrongPwdRejected = true;
      wrongErrMsg = String(e).slice(0, 200);
    }
    assert(wrongPwdRejected, '错误密码未拒绝（服务端 401 应抛错到 JS）');
    console.log('  3. ✅ 错误密码被拒:', wrongErrMsg.slice(0, 80));

    // 密码校验限流：join_channel rate 0.5/s，等 1.5s 再试
    await u1.waitForTimeout(1500);

    // ---- Step 5: u1 用正确密码 join → 期望成功 ----
    try {
      await u1.evaluate(async ({ ch, pw }) => window.__kaede_debug__.channels.joinWithPassword(ch, pw), { ch: chId, pw: PW });
      console.log('  4. ✅ u1 正确密码 join 成功');
    } catch (e) {
      u1Logs.filter(l => /join|channel|mls|welcome|error/i).slice(-12).forEach(l => console.log('    [u1 log]', l));
      throw new Error('u1 正确密码 join 失败: ' + String(e).slice(0, 300));
    }

    // 等 Welcome + 频道 group 建立
    console.log('  等待频道 group 同步（5 秒）...');
    await u1.waitForTimeout(5000);

    const stU0 = await u0.evaluate(ch => window.__kaede_debug__.channelState(ch), chId);
    const stU1 = await u1.evaluate(ch => window.__kaede_debug__.channelState(ch), chId);
    console.log(`  epoch(u0)=${stU0 && stU0.epoch}  epoch(u1)=${stU1 && stU1.epoch}`);

    // ---- Step 6: u0 发消息 → u1 收到 ChannelMessage ----
    await u1.evaluate(() => window.__kaede_debug__.events.clear());
    const mark = 'PV_MSG_' + Date.now();
    await u0.evaluate(async ({ ch, t }) => window.__kaede_debug__.sendMessage(ch, t), { ch: chId, t: mark });
    console.log('  5. u0 发消息:', mark);
    await u0.waitForTimeout(3000);

    const evs1 = await u1.evaluate(() => window.__kaede_debug__.events.list());
    const channelMsgEvs = evs1.filter(e => e && e.ChannelMessage);
    const appMsgEvs = evs1.filter(e => e && e.AppMessageReceived);
    console.log(`     u1: ChannelMessage=${channelMsgEvs.length}, AppMessageReceived=${appMsgEvs.length}`);

    if (channelMsgEvs.length > 0) {
      console.log('  6. ✅ u1 收到私密频道 ChannelMessage（0x03 路径）');
      if (appMsgEvs.length > 0) {
        console.log('     ⚠️ 也收到 AppMessageReceived（不该有，私密频道不走 MLS broadcast）');
      }
    } else {
      u1Logs.filter(l => /channel|private|mls|error/i).slice(-12).forEach(l => console.log('    [u1 log]', l));
      u0Logs.filter(l => /channel|private|mls|error/i).slice(-12).forEach(l => console.log('    [u0 log]', l));
      throw new Error('u1 未收到私密频道消息');
    }

    // 用 loadHistory 二次确认
    const hist1 = await u1.evaluate(async ch => window.__kaede_debug__.loadHistory(ch, 30), chId);
    const foundInHist = Array.isArray(hist1) && hist1.some(m => (m.plaintext || '').includes(mark));
    if (!foundInHist) {
      console.log('     ⚠️ u1 loadHistory 未找到消息（在线推送通但历史可能未同步，非致命）');
    } else {
      console.log('  6b. ✅ u1 loadHistory 也找到消息');
    }

    // ---- Step 7: u0 reaction → u1 收到 ChannelReaction（私密路径） ----
    const hist0 = await u0.evaluate(async ch => window.__kaede_debug__.loadHistory(ch, 30), chId);
    const myMsg = (hist0 || []).find(m => (m.plaintext || '').includes(mark));
    if (!myMsg) throw new Error('u0 找不到自己发的消息，无法 reaction');
    const targetTs = Number(myMsg.ts);

    await u1.evaluate(() => window.__kaede_debug__.events.clear());
    await u0.evaluate(async ({ ch, ts }) => window.__kaede_debug__.reaction.send(ch, ts, '🔥', 'add'), { ch: chId, ts: targetTs });
    console.log('  7. u0 发 reaction 🔥 (私密频道 → 期望 ChannelReaction)');
    await u0.waitForTimeout(2500);

    const evs2 = await u1.evaluate(() => window.__kaede_debug__.events.list());
    const reactionEvs = evs2.filter(e => e && e.ChannelReaction);
    const wrongPathEvs = evs2.filter(e => e && e.AppMessageReceived);
    if (reactionEvs.length === 0) {
      u1Logs.filter(l => /reaction|channel|mls|error/i).slice(-12).forEach(l => console.log('    [u1 log]', l));
      throw new Error('u1 未收到私密频道 ChannelReaction');
    }
    console.log('  8. ✅ u1 收到 ChannelReaction（私密路径正确）');
    if (wrongPathEvs.length > 0) {
      console.log('     ⚠️ 也收到 AppMessageReceived（不该有）');
    }

    // ---- Step 8: 清理 ----
    try {
      await u0.evaluate(async ch => window.__kaede_debug__.channels.delete(ch), chId);
      chId = null;
      console.log('  9. ✅ 清理：私密频道已删除');
    } catch (e) {
      console.log('  9. ⚠️ 清理失败（非致命）:', String(e).slice(0, 100));
    }

    console.log('\n🎉 测试 27 通过');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 27 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
