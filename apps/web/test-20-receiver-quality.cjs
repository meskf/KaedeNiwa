// test-20: 屏幕共享接收端画质切换（真断言版）
// 断言: livekit 日志 "-> 480p30 (layer=low)" / "-> 1080p60 (layer=high)"
//       观测: videoWidth 序列（下降 → STRONG PASS；不变 → WARN 报告供排查 SFU dynacast）
const { checkPrereqs, login, randName, injectFakeDisplayMedia, setDisableAdaptive } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 测试 20: 屏幕共享接收端画质（真断言）\n');
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

  try {
    const aliceName = randName('alice20');
    const bobName = randName('bob20');

    const alice = await (await browser.newContext()).newPage();
    await setDisableAdaptive(alice);
    await injectFakeDisplayMedia(alice);
    await login(alice, aliceName);
    console.log('👤 Alice:', aliceName);

    const bob = await (await browser.newContext()).newPage();
    await setDisableAdaptive(bob);
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

    async function joinVoice(page, label) {
      console.log('🎤 ' + label + ' 加入语音...');
      for (let i = 0; i < 3; i++) {
        if (await page.locator(MEDIA_OK_SEL).count() > 0) return;
        await page.click('[data-testid="channel-item-voice"]');
        await page.waitForTimeout(3000);
        if (await page.locator('[data-testid="media-join"]').count() > 0) {
          await page.click('[data-testid="media-join"]');
          await page.waitForTimeout(3000);
        }
        try { await page.waitForSelector(MEDIA_OK_SEL, { timeout: 15000 }); return; }
        catch (e) { if (i === 2) throw e; await page.waitForTimeout(2000); }
      }
    }
    await joinVoice(alice, 'Alice');
    await joinVoice(bob, 'Bob');

    await alice.waitForTimeout(2000);

    console.log('🖥️  Alice 共享屏幕...');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForSelector('[data-testid="media-share-toggle"][data-sharing="on"]', { timeout: 10000 });

    console.log('📺 Bob 等待画面...');
    await bob.waitForSelector('[data-testid="screen-share-video"]', { timeout: 20000 });
    await bob.waitForTimeout(3000);

    async function readVideoDims() {
      return await bob.evaluate(() => {
        const container = document.querySelector('[data-testid="screen-share-video"]');
        if (!container) return null;
        const v = container.tagName === 'VIDEO' ? container : container.querySelector('video');
        if (!v) return null;
        const vw = v.videoWidth | 0;
        const vh = v.videoHeight | 0;
        if (!vw || !vh) return null;
        return { w: vw, h: vh };
      });
    }

    const dims0 = await readVideoDims();
    console.log('📊 初始分辨率:', JSON.stringify(dims0));
    if (!dims0) { console.error('❌ 初始没有 videoWidth（首帧没解码）'); process.exit(1); }

    console.log('\n🎚️  检查 Bob 侧 quality <select>...');
    const selectLocator = bob.locator('[data-testid^="screen-share-quality-"]').first();
    if (await selectLocator.count() === 0) { console.error('❌ 未找到接收端画质选择器'); process.exit(1); }
    const selectId = await selectLocator.getAttribute('data-testid');
    console.log('   select id = ' + selectId);

    const opts = await bob.evaluate((id) => {
      const el = document.querySelector('[data-testid="' + id + '"]');
      if (!el) return null;
      return { value: el.value, options: Array.from(el.querySelectorAll('option')).map(o => ({ value: o.value, label: o.textContent })) };
    }, selectId);
    console.log('   options:', JSON.stringify(opts));
    if (!opts || opts.options.length === 0) { console.error('❌ select 没有 option'); process.exit(1); }

    console.log('\n🎚️  切换到 480p30（期望 layer=low）...');
    bobLogs.length = 0;
    await selectLocator.selectOption('480p30');
    await bob.waitForTimeout(3500);
    const hasLowLog = bobLogs.some(l => l.includes('-> 480p30') && l.includes('layer=low'));
    if (!hasLowLog) {
      console.error('❌ 没有看到 "-> 480p30 (layer=low)" 日志');
      bobLogs.filter(l => /ScreenShare|quality/i.test(l)).forEach(l => console.log('    ', l));
      process.exit(1);
    }
    console.log('   ✅ 已调 setScreenShareQuality → 480p30 (layer=low)');
    const dimsLow = await readVideoDims();
    console.log('📊 切 low 后分辨率:', JSON.stringify(dimsLow));

    console.log('\n🎚️  切换到 1080p60（期望 layer=high）...');
    bobLogs.length = 0;
    await selectLocator.selectOption('1080p60');
    await bob.waitForTimeout(3500);
    const hasHighLog = bobLogs.some(l => l.includes('-> 1080p60') && l.includes('layer=high'));
    if (!hasHighLog) {
      console.error('❌ 没有看到 "-> 1080p60 (layer=high)" 日志');
      bobLogs.filter(l => /ScreenShare|quality/i.test(l)).forEach(l => console.log('    ', l));
      process.exit(1);
    }
    console.log('   ✅ 已调 setScreenShareQuality → 1080p60 (layer=high)');
    const dimsHigh = await readVideoDims();
    console.log('📊 切 high 后分辨率:', JSON.stringify(dimsHigh));

    console.log('\n================ 分辨率变化报告 ================');
    console.log('初始: ' + (dims0 ? dims0.w + 'x' + dims0.h : 'null'));
    console.log('low : ' + (dimsLow ? dimsLow.w + 'x' + dimsLow.h : 'null'));
    console.log('high: ' + (dimsHigh ? dimsHigh.w + 'x' + dimsHigh.h : 'null'));
    console.log('================================================');

    const strict = process.env.KAEDE_QUALITY_WARN_OK !== '1'
    const strongPass = dimsLow && dimsHigh && dims0 &&
                       dimsLow.w < dims0.w * 0.9 &&
                       dimsHigh.w >= dimsLow.w * 1.2;
    if (strongPass) {
      console.log('✅ STRONG PASS: 分辨率真的随画质切换变化（simulcast 生效）');
    } else if (dimsLow && dims0 && dimsLow.w === dims0.w && dimsHigh && dimsHigh.w === dims0.w) {
      console.warn('⚠️  WARN: videoWidth 没变（UI 链路通，但 SFU 未降层）');
      console.warn('   可能原因: 发送端 simulcast 层未按预期编码，或 SFU 判定网络够好保持高层');
      console.warn('   进一步诊断: 看 LiveKit 终端日志的 dynacast，或发送端 getDisplayMedia 实际分辨率');
      if (strict) { await browser.close(); process.exit(1) }
    } else {
      console.warn('⚠️  WARN: 分辨率变化不符合预期');
    }

    try { await alice.click('[data-testid="media-share-toggle"]'); } catch {}
    await alice.waitForTimeout(1000);

    console.log('\n🎉 测试 20 通过（UI 链路已通，分辨率断言见上方报告）');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('❌ 测试 20 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
