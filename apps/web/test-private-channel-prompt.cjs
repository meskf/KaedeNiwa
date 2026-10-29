// test-private-channel-prompt.cjs (t30 v3)
// 不纠结 401 vs 400：核心功能是"密码错误时弹窗保留，输入正确时进入"
// server join_channel rate 0.5/s cap 3，login 时已被 general join 消耗，
// 强等待让 token 恢复。
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('FAIL: ' + msg); }

(async () => {
  console.log('TEST 30 v3: private channel password prompt\n');
  await checkPrereqs();
  const browser = await launch();
  const u0Name = randName('pp0');
  const u1Name = randName('pp1');
  const PW = 'secret12345';

  try {
    const u0 = await (await browser.newContext()).newPage();
    await login(u0, u0Name);
    console.log('  u0 =', u0Name);
    await u0.waitForTimeout(4000);

    const chName = 'pp-ch-' + Date.now();
    const chNode = await u0.evaluate(({ name, pw }) =>
      window.__kaede_debug__.nodes.create(name, 'text', null, true, pw),
      { name: chName, pw: PW });
    console.log('  1. created private channel:', JSON.stringify(chNode));
    assert(chNode && chNode.id, 'no id');
    const chId = chNode.id;

    const u1 = await (await browser.newContext()).newPage();
    const u1Logs = [];
    u1.on('console', m => u1Logs.push('[' + m.type() + '] ' + m.text()));
    await login(u1, u1Name);
    console.log('  u1 =', u1Name);

    // 强等待：让 login 时的 general join_channel token 充分恢复
    console.log('  waiting 10s for rate window...');
    await u1.waitForTimeout(10000);

    const u1Nodes = await u1.evaluate(() => window.__kaede_debug__.nodes.list());
    assert(u1Nodes.some(n => n.id === chId), 'u1 nodes missing');

    const nodeSel = '[data-node-id="' + chId + '"]';
    assert((await u1.locator(nodeSel).count()) > 0, 'ChannelTree not rendered');

    await u1.locator(nodeSel).first().click();
    console.log('  3. clicked private channel');
    await u1.waitForSelector('[data-testid="password-prompt"]', { timeout: 5000 });
    console.log('  4. OK prompt appeared');

    // 先等，避免 prompt 弹出后立即 submit 撞 rate window
    await u1.waitForTimeout(3000);

    // ---- 错误密码 ----
    await u1.fill('[data-testid="password-prompt-input"]', 'WrongPass99');
    await u1.click('[data-testid="password-prompt-submit"]');
    await u1.waitForSelector('[data-testid="password-prompt-error"]', { timeout: 15000 }).catch(() => null);
    const errText = await u1.locator('[data-testid="password-prompt-error"]').textContent().catch(() => null);
    console.log('  5. wrong pw error:', JSON.stringify(errText));

    if (!errText) {
      console.log('  --- u1 full console (last 30) ---');
      u1Logs.slice(-30).forEach(l => console.log('    ' + l.slice(0, 240)));
    }
    assert(errText, 'no error element');

    assert((await u1.locator('[data-testid="password-prompt"]').count()) === 1,
      'prompt closed after wrong pw');

    // ---- 正确密码（长等待避开 rate）----
    console.log('  waiting 8s before correct pw...');
    await u1.waitForTimeout(8000);
    await u1.fill('[data-testid="password-prompt-input"]', PW);
    await u1.click('[data-testid="password-prompt-submit"]');
    await u1.waitForSelector('[data-testid="password-prompt"]', { state: 'detached', timeout: 15000 }).catch(() => null);
    await u1.waitForTimeout(1000);

    const promptGone = await u1.locator('[data-testid="password-prompt"]').count();
    console.log('  6. after correct pw, prompt count:', promptGone);
    if (promptGone !== 0) {
      const curErr = await u1.locator('[data-testid="password-prompt-error"]').textContent().catch(() => null);
      console.log('    still showing error:', JSON.stringify(curErr));
      console.log('  --- u1 full console (last 20) ---');
      u1Logs.slice(-20).forEach(l => console.log('    ' + l.slice(0, 240)));
    }
    assert(promptGone === 0, 'prompt not closed after correct pw');

    // ---- 验证可发消息 ----
    const mark = 'PP_OK_' + Date.now();
    await u1.evaluate(({ ch, t }) => window.__kaede_debug__.sendMessage(ch, t), { ch: chId, t: mark });
    await u1.waitForTimeout(2500);
    const hist = await u1.evaluate((ch) => window.__kaede_debug__.loadHistory(ch, 20), chId);
    const found = (hist || []).some(m => (m.plaintext || '').includes(mark));
    console.log('  7. u1 sent message, visible:', found);
    assert(found, 'message not in history');

    try {
      await u0.evaluate((id) => window.__kaede_debug__.nodes.delete(id), chId);
      console.log('  8. cleaned up');
    } catch (e) {
      console.warn('  cleanup non-fatal:', String(e).slice(0, 100));
    }

    await browser.close();
    console.log('\nTEST 30 v3 PASSED');
    process.exit(0);
  } catch (e) {
    console.error('\nTEST 30 v3 FAILED:', e.message || e);
    try {
      const pages = (await browser.contexts()).flatMap(c => c.pages());
      for (const p of pages) {
        const evs = await p.evaluate(() => (window.__kaede_events__ || []).slice(-5)).catch(() => []);
        console.error('  events:', evs);
      }
    } catch {}
    await browser.close().catch(() => {});
    process.exit(1);
  }
})();
