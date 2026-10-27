const { checkPrereqs, login, randName, typeMessage } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 测试 13: pending 队列（断网发消息，重连自动重试）');
  await checkPrereqs();
  const browser = await launch();
  try {
    const u0Name = randName('u0');
    const u1Name = randName('u1');

    const u0Ctx = await browser.newContext();
    const u0 = await u0Ctx.newPage();
    await login(u0, u0Name);
    console.log('  u0:', u0Name);

    const u1 = await (await browser.newContext()).newPage();
    await login(u1, u1Name);
    console.log('  u1:', u1Name);

    await u0.waitForTimeout(5000);
    console.log('  ✅ 2 人已进 #general');

    // 断网（HTTP + WS 都断）
    console.log('  🔌 u0 断网...');
    await u0Ctx.setOffline(true);

    // 发 3 条消息 → 全部进 pending
    const msgs = ['PENDING_A_' + Date.now(), 'PENDING_B_' + Date.now(), 'PENDING_C_' + Date.now()];
    for (const m of msgs) {
      await typeMessage(u0, m);
      await u0.waitForTimeout(300);
    }
    console.log('  ✅ u0 断网期间发了 3 条消息');

    // 检查 pending 状态：应该有 3 条 "发送失败" 或 "发送中"
    await u0.waitForTimeout(1000);
    const pendingCount = await u0.locator('[data-testid="pending-msg"][data-status="failed"]').count() + await u0.locator('[data-testid="pending-msg"][data-status="sending"]').count();
    console.log('  pending 渲染数量:', pendingCount);

    // 恢复网络
    console.log('  🌐 u0 恢复网络...');
    await u0Ctx.setOffline(false);

    // 等重连 + 自动重试（5 秒一轮 + 20 秒缓冲）
    console.log('  ⏳ 等自动重试（25 秒）...');
    await u0.waitForTimeout(25000);

    // 断言：u1 收到全部 3 条
    let received = 0;
    for (const m of msgs) {
      try {
        await u1.waitForSelector('text=' + m, { timeout: 8000 });
        received++;
        console.log('  ✅ u1 收到:', m);
      } catch (e) {
        console.log('  ❌ u1 未收到:', m);
      }
    }

    // 断言：u0 的 pending 应该清空
    const remainingPending = await u0.locator('[data-testid="pending-msg"][data-status="failed"]').count() + await u0.locator('[data-testid="pending-msg"][data-status="sending"]').count();
    console.log('  重连后剩余 pending:', remainingPending);

    if (received === 3 && remainingPending === 0) {
      console.log('\n🎉 测试 13 通过：pending 队列自动重试成功');
    } else {
      console.log('\n❌ 测试 13 失败：收到 ' + received + '/3，剩余 pending ' + remainingPending);
      process.exit(1);
    }
  } catch (e) {
    console.error('❌ 测试 13 异常:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
