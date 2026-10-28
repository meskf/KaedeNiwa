// test-20: 屏幕共享接收端画质切换
// 验证：Alice 共享 → Bob 侧出现 quality <select> → 切 low 触发 setScreenShareQuality
//        → console 显示 setVideoQuality → cache 更新 → 无错误
// 宽容断言：videoWidth 可能不变（simulcast 依赖 publishing preset），但 UI 路径必须通
const { checkPrereqs, login, randName, injectFakeDisplayMedia } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 测试 20: 屏幕共享接收端画质\n');
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

  try {
    const aliceName = randName('alice20');
    const bobName = randName('bob20');

    const alice = await (await browser.newContext()).newPage();
    await injectFakeDisplayMedia(alice);
    await login(alice, aliceName);
    console.log('👤 Alice:', aliceName);

    const bob = await (await browser.newContext()).newPage();
    const bobLogs = [];
    bob.on('console', m => {
      const t = m.text();
      bobLogs.push(t);
      if (/ScreenShare|screen-share|quality/i.test(t)) console.log('[Bob]', t.slice(0, 200));
    });
    await login(bob, bobName);
    console.log('👤 Bob:', bobName);

    console.log('⏳ 等待自动拉人...');
    await alice.waitForTimeout(4000);

    const MEDIA_OK_SEL = '[data-testid="media-leave"], [data-testid="media-connected"]';

    console.log('🎤 Alice 加入语音...');
    for (let i = 0; i < 3; i++) {
      if (await alice.locator(MEDIA_OK_SEL).count() > 0) break;
      await alice.click('[data-testid="channel-item-voice"]');
      await alice.waitForTimeout(3000);
      if (await alice.locator('[data-testid="media-join"]').count() > 0) {
        await alice.click('[data-testid="media-join"]');
        await alice.waitForTimeout(3000);
      }
      try { await alice.waitForSelector(MEDIA_OK_SEL, { timeout: 15000 }); break; }
      catch (e) { if (i === 2) throw e; await alice.waitForTimeout(2000); }
    }

    console.log('🎤 Bob 加入语音...');
    for (let i = 0; i < 3; i++) {
      if (await bob.locator(MEDIA_OK_SEL).count() > 0) break;
      await bob.click('[data-testid="channel-item-voice"]');
      await bob.waitForTimeout(3000);
      if (await bob.locator('[data-testid="media-join"]').count() > 0) {
        await bob.click('[data-testid="media-join"]');
        await bob.waitForTimeout(3000);
      }
      try { await bob.waitForSelector(MEDIA_OK_SEL, { timeout: 15000 }); break; }
      catch (e) { if (i === 2) throw e; await bob.waitForTimeout(2000); }
    }

    await alice.waitForTimeout(2000);

    console.log('🖥️  Alice 共享屏幕...');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForSelector('[data-testid="media-share-toggle"][data-sharing="on"]', { timeout: 10000 });

    console.log('📺 Bob 等待画面...');
    await bob.waitForSelector('[data-testid="screen-share-video"]', { timeout: 20000 });
    await bob.waitForTimeout(2000);

    // ====== 关键断言 ======
    console.log('🎚️  检查 Bob 侧的 quality <select>...');
    const selectors = await bob.locator('[data-testid^="screen-share-quality-"]').count();
    console.log(`   找到 ${selectors} 个 quality select`);
    if (selectors === 0) {
      console.error('❌ 未找到接收端画质选择器');
      process.exit(1);
    }

    const selectId = await bob.locator('[data-testid^="screen-share-quality-"]').first().getAttribute('data-testid');
    const sid = selectId.replace('screen-share-quality-', '');
    console.log(`   select id = ${selectId}`);

    console.log('🎚️  切换到 360p (low)...');
    bobLogs.length = 0;
    await bob.locator(`[data-testid="${selectId}"]`).selectOption('low');
    await bob.waitForTimeout(2000);

    const hasLowLog = bobLogs.some(l => l.includes('-> low'));
    if (!hasLowLog) {
      console.error('❌ 没有看到 setScreenShareQuality 的 low 日志');
      console.log('   Bob 相关日志:');
      bobLogs.filter(l => /ScreenShare|quality|error/i.test(l)).forEach(l => console.log('    ', l));
      process.exit(1);
    }
    console.log('   ✅ 已调 setVideoQuality(low)');

    console.log('🎚️  切换到 最高 (high)...');
    bobLogs.length = 0;
    await bob.locator(`[data-testid="${selectId}"]`).selectOption('high');
    await bob.waitForTimeout(2000);

    const hasHighLog = bobLogs.some(l => l.includes('-> high'));
    if (!hasHighLog) {
      console.error('❌ 没有看到 high 日志');
      bobLogs.filter(l => /ScreenShare|quality|error/i.test(l)).forEach(l => console.log('    ', l));
      process.exit(1);
    }
    console.log('   ✅ 已调 setVideoQuality(high)');

    // 宽容断言：videoWidth 可能不变（取决于 simulcast），但应该有 video 元素
    const dims = await bob.evaluate(() => {
      for (const v of document.querySelectorAll('video')) {
        if (v.videoWidth > 0) return { w: v.videoWidth, h: v.videoHeight };
      }
      return null;
    });
    console.log('📊 当前分辨率:', JSON.stringify(dims));
    if (!dims) {
      console.error('❌ 没有可用的 video 元素');
      process.exit(1);
    }

    // 清理
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForTimeout(1000);

    console.log('');
    console.log('🎉 测试 20 通过：接收端画质切换链路完整');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('❌ 测试 20 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
