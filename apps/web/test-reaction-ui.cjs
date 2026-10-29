// test-reaction-ui.cjs (t29): Reaction UI 端到端
//
// 验证：
//   1. 右键消息弹出 context menu（含 8 emoji 快捷行）
//   2. 点 👍 → 双方 DOM 出现 [data-testid="reaction-pill-👍"][data-count="1"][data-mine]
//   3. 点 pill → 双方消失
//
// 覆盖 general 频道的 0x03 ChannelMessage 密文路径 reaction

const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }

(async () => {
  console.log('🧪 测试 29: Reaction UI (general 频道)\n');
  await checkPrereqs();
  const browser = await launch();
  const u0Name = randName('rx0');
  const u1Name = randName('rx1');
  const CH = 'general';
  const EMOJI = '👍';

  try {
    const u0 = await (await browser.newContext()).newPage();
    const u1 = await (await browser.newContext()).newPage();
    const u0Logs = [], u1Logs = [];
    u0.on('console', m => u0Logs.push('[' + m.type() + '] ' + m.text()));
    u1.on('console', m => u1Logs.push('[' + m.type() + '] ' + m.text()));

    await login(u0, u0Name); console.log('  u0 =', u0Name);
    await login(u1, u1Name); console.log('  u1 =', u1Name);

    console.log('  等待 general MLS group 建好（12s）...');
    await u0.waitForTimeout(12000);

    // ---- u0 发消息 ----
    const mark = 'RX_' + Date.now();
    await u0.evaluate(({ ch, t }) => window.__kaede_debug__.sendMessage(ch, t), { ch: CH, t: mark });
    console.log('  1. u0 发消息:', mark);
    await u0.waitForTimeout(3000);

    // ---- u1 侧找到该消息行 ----
    const rowSel = `[data-testid="message-row"]:has-text("${mark}")`;
    await u1.waitForSelector(rowSel, { timeout: 10000 });
    const rowLoc = u1.locator(rowSel).first();
    const ts = await rowLoc.getAttribute('data-msg-ts');
    console.log('  2. u1 找到消息行，ts =', ts);
    assert(ts && ts !== 'null', 'data-msg-ts 为空');

    // ---- 右键 ----
    await rowLoc.click({ button: 'right' });
    await u1.waitForSelector('[data-testid="message-context-menu"]', { timeout: 5000 });
    console.log('  3. ✅ 右键菜单出现');

    // ---- 断言 emoji 快捷行 ----
    const emojiBtnCount = await u1.locator('[data-testid="message-context-emojis"] [data-emoji]').count();
    console.log('     快捷 emoji 数量 =', emojiBtnCount);
    assert(emojiBtnCount === 8, '预期 8 个快捷 emoji，实际 ' + emojiBtnCount);

    // ---- 点击 👍 ----
    const thumbSel = `[data-testid="message-context-emojis"] [data-emoji="${EMOJI}"]`;
    await u1.locator(thumbSel).first().click();
    console.log('  4. u1 点击 👍');
    await u0.waitForTimeout(3000);

    // ---- 双方断言 ----
    const pillSel = `[data-testid="reaction-pill-${EMOJI}"]`;
    await u1.waitForSelector(pillSel, { timeout: 8000 });
    const pill1 = await u1.locator(pillSel).first().evaluate(el => ({
      count: el.getAttribute('data-count'),
      mine: el.getAttribute('data-mine'),
    }));
    console.log('     u1 pill:', JSON.stringify(pill1));
    assert(pill1.count === '1', 'u1 count 应为 1，实际 ' + pill1.count);
    assert(pill1.mine === '1', 'u1 mine 应为 1（自己是投手），实际 ' + pill1.mine);

    await u0.waitForSelector(pillSel, { timeout: 8000 });
    const pill0 = await u0.locator(pillSel).first().evaluate(el => ({
      count: el.getAttribute('data-count'),
      mine: el.getAttribute('data-mine'),
    }));
    console.log('     u0 pill:', JSON.stringify(pill0));
    assert(pill0.count === '1', 'u0 count 应为 1，实际 ' + pill0.count);
    assert(pill0.mine === '0', 'u0 mine 应为 0（他人投的），实际 ' + pill0.mine);

    console.log('  5. ✅ 双方都看到 👍 count=1');

    // ---- u1 点击 pill 取消 ----
    await u1.locator(pillSel).first().click();
    console.log('  6. u1 点击 pill 取消 👍');
    await u0.waitForTimeout(3000);

    const pill1After = await u1.locator(pillSel).count();
    const pill0After = await u0.locator(pillSel).count();
    console.log(`     u1 剩余 pill 数 = ${pill1After}, u0 剩余 = ${pill0After}`);
    assert(pill1After === 0, 'u1 取消后 pill 应消失，剩 ' + pill1After);
    assert(pill0After === 0, 'u0 侧 pill 应消失，剩 ' + pill0After);
    console.log('  7. ✅ 双方 👍 都消失');

    // ---- 再次投递（第二轮验证幂等） ----
    await rowLoc.click({ button: 'right' });
    await u1.waitForSelector('[data-testid="message-context-menu"]', { timeout: 5000 });
    await u1.locator(thumbSel).first().click();
    await u0.waitForTimeout(3000);
    const pill1b = await u1.locator(pillSel).first().evaluate(el => el.getAttribute('data-count'));
    assert(pill1b === '1', '第二轮 count 应为 1，实际 ' + pill1b);
    console.log('  8. ✅ 第二轮重投正常（count=1）');

    await browser.close();
    console.log('\n🎉 测试 29 通过');
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 29 失败:', e.message || e);
    try {
      const pages = (await browser.contexts()).flatMap(c => c.pages());
      if (pages[0]) {
        const logs = await pages[0].evaluate(() => (window.__kaede_events__ || []).slice(-8)).catch(() => []);
        console.error('  last events (page[0]):', logs);
      }
    } catch {}
    await browser.close().catch(() => {});
    process.exit(1);
  }
})();
