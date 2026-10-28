// test-28: 跨用户消息历史同步（同 epoch 语义）
// 关键：MLS 前向安全下，后加入的成员无法解密加入前的历史（epoch 不同 → key 不同）。
// 本测试覆盖的是「u0 和 u1 都已在 general group 内」→ u0 发 10 条 → 双方 loadHistory 都能拿到。
// 跨 epoch 历史不可读是 MLS 的设计取舍，记录在 HANDOFF。
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }

(async () => {
  console.log('🧪 测试 28: 跨用户消息历史同步（同 epoch）\n');
  await checkPrereqs();
  const browser = await launch();
  const u0Name = randName('hist0');
  const u1Name = randName('hist1');
  const CH = 'general';
  const N = 10;
  try {
    const u0 = await (await browser.newContext()).newPage();
    const u1 = await (await browser.newContext()).newPage();
    const u0Logs = [], u1Logs = [];
    u0.on('console', m => u0Logs.push(m.text()));
    u1.on('console', m => u1Logs.push(m.text()));

    // Step 1: 先 u0 登录
    await login(u0, u0Name); console.log('  u0 =', u0Name);
    await u0.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.sendMessage), null, { timeout: 10000 });

    // Step 2: 再 u1 登录（触发 u0 add_member(u1) → epoch 前进）
    await login(u1, u1Name); console.log('  u1 =', u1Name);
    await u1.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.loadHistory), null, { timeout: 10000 });

    // Step 3: 关键——等双方 epoch 同步完成（u0 add u1 后 u1 收 Welcome + Commit → epoch=2）
    //         这时 u0 和 u1 在同一 epoch
    console.log('  等待 epoch 同步（8 秒，容错）...');
    await u0.waitForTimeout(8000);

    // 校验双方 epoch 一致
    const st0 = await u0.evaluate(async (ch) => window.__kaede_debug__.channelState(ch), CH);
    const st1 = await u1.evaluate(async (ch) => window.__kaede_debug__.channelState(ch), CH);
    const ep0 = st0 && st0.epoch;
    const ep1 = st1 && st1.epoch;
    console.log(`  3. epoch(u0) = ${ep0}  epoch(u1) = ${ep1}`);
    if (typeof ep0 === 'number' && typeof ep1 === 'number') {
      assert(ep0 === ep1, `epoch 不一致：u0=${ep0}, u1=${ep1}（MLS group 未对齐，测试不成立）`);
      console.log('     ✅ 双方 epoch 一致');
    } else {
      console.log('     ⚠️ 软断言：channelState 未返回 epoch，跳过严格对齐检查');
    }

    // Step 4: u0 发 N 条唯一消息（此刻双方同 epoch）
    const mark = 'HIST_' + Date.now();
    for (let i = 0; i < N; i++) {
      const t = `${mark}_${i}`;
      await u0.evaluate(async ({ ch, t }) => window.__kaede_debug__.sendMessage(ch, t), { ch: CH, t });
      await u0.waitForTimeout(200);
    }
    console.log(`  4. ✅ u0 已发 ${N} 条（前缀 ${mark}）`);
    await u0.waitForTimeout(3000);

    // Step 5: u1 loadHistory
    const hist1 = await u1.evaluate(async (ch) => window.__kaede_debug__.loadHistory(ch, 100), CH);
    assert(Array.isArray(hist1), 'loadHistory(u1) 应返回数组');
    console.log(`  5. ✅ u1 拿到 ${hist1.length} 条历史`);

    // Step 6: u0 也 loadHistory（自证：自己发的能解）
    const hist0 = await u0.evaluate(async (ch) => window.__kaede_debug__.loadHistory(ch, 100), CH);
    assert(Array.isArray(hist0), 'loadHistory(u0) 应返回数组');
    console.log(`  6. ✅ u0 拿到 ${hist0.length} 条历史`);

    // Step 7: 严格断言 N 条全部命中（双方各自独立检查）
    const checkAll = (hist, label) => {
      const found = new Set();
      for (const m of hist) {
        const match = (m.plaintext || '').match(new RegExp(mark + '_(\\d+)'));
        if (match) found.add(parseInt(match[1], 10));
      }
      const missing = [];
      for (let i = 0; i < N; i++) if (!found.has(i)) missing.push(i);
      if (missing.length > 0) {
        throw new Error(`${label} 缺少 ${missing.length}/${N} 条：${missing.join(',')}（共 ${hist.length} 条）`);
      }
      return found.size;
    };

    try {
      const n1 = checkAll(hist1, 'u1');
      console.log(`  7. ✅ u1 命中 ${n1}/${N}`);
    } catch (e) {
      u1Logs.filter(l => /hist|channel|mls|error|application|decrypt/i.test(l)).slice(-15).forEach(l => console.log('    [u1 log]', l));
      throw e;
    }

    try {
      const n0 = checkAll(hist0, 'u0');
      console.log(`  8. ✅ u0 命中 ${n0}/${N}`);
    } catch (e) {
      u0Logs.filter(l => /hist|channel|mls|error|application|decrypt/i.test(l)).slice(-15).forEach(l => console.log('    [u0 log]', l));
      throw e;
    }

    console.log('\n🎉 测试 28 通过（同 epoch 跨用户历史）');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 28 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
