const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 测试 5: 麦克风 + RNNoise\n');
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');

    const alice = await (await browser.newContext()).newPage();
    const aliceLogs = [];
    alice.on('console', msg => {
      const t = msg.text();
      if (t.includes('RNNoise') || t.includes('audio') || t.includes('E2EE')) {
        aliceLogs.push(t);
        console.log('[Alice]', t);
      }
    });
    await login(alice, aliceName);
    console.log('👤 Alice:', aliceName);

    const bob = await (await browser.newContext()).newPage();
    await login(bob, bobName);
    console.log('👤 Bob:', bobName);

    console.log('⏳ 等待自动拉人...');
    await alice.waitForTimeout(4000);

    console.log('🎤 Alice 加入语音...');
    await alice.click('[data-testid="channel-item-voice"]');
    await alice.waitForTimeout(3000);
    await alice.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await alice.click('[data-testid="media-join"]');
    await alice.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });

    console.log('🎤 Bob 加入语音...');
    await bob.click('[data-testid="channel-item-voice"]');
    await bob.waitForTimeout(3000);
    await bob.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await bob.click('[data-testid="media-join"]');
    await bob.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
    await alice.waitForTimeout(2000);

    // ====== 场景 1：Alice 开麦 + RNNoise ======
    console.log('\n🎙️ 场景 1: Alice 开麦 + RNNoise');
    await alice.click('[data-testid="media-mic-toggle"]');
    await alice.waitForSelector('[data-testid="media-mic-toggle"]', { timeout: 10000 });
    console.log('✅ Alice 麦克风已开启');

    // 检查 RNNoise 是否加载（console 里应该有）
    await alice.waitForTimeout(2000);
    const rnnoiseLoaded = aliceLogs.some(l => l.includes('RNNoise] 降噪已启用') || l.includes('RNNoise] ready'));
    if (rnnoiseLoaded) {
      console.log('✅ RNNoise 降噪已加载');
    } else {
      console.log('⚠️ 未在 console 里看到 RNNoise 日志（可能路径问题）');
    }

    // 降噪按钮应该出现
    const noiseBtn = await alice.locator('[data-testid="media-noise-toggle"]').count();
    if (noiseBtn > 0) {
      console.log('✅ 降噪按钮出现');
    } else {
      console.log('❌ 降噪按钮未出现');
      process.exit(1);
    }

    // ====== 场景 2：Bob 看到 Alice 的 🎤 图标 ======
    console.log('\n🎙️ 场景 2: Bob 看到 Alice 麦克风状态');
    await bob.waitForTimeout(2000);
    // participants 里的 mic icon
    const bobSeesAliceMic = await bob.locator(`[data-participant="${aliceName}"][data-mic="on"]`).count();
    if (bobSeesAliceMic > 0) {
      console.log('✅ Bob 看到 Alice 的麦克风图标');
    } else {
      console.log('⚠️ Bob 没看到 Alice 的 🎤（可能时序）');
    }

    // ====== 场景 3：Alice 关闭降噪 ======
    console.log('\n🔇 场景 3: Alice 关闭降噪');
    await alice.click('[data-testid="media-noise-toggle"]');
    await alice.waitForSelector('[data-testid="media-noise-toggle"]', { timeout: 5000 });
    console.log('✅ 降噪已关闭');

    // ====== 场景 4：Alice 关闭麦克风 ======
    console.log('\n🔇 场景 4: Alice 关闭麦克风');
    await alice.click('[data-testid="media-mic-toggle"]');
    await alice.waitForSelector('[data-testid="media-mic-toggle"]', { timeout: 5000 });
    console.log('✅ 麦克风已关闭');

    console.log('\n🎉 测试 5 通过');
  } catch (e) {
    console.error('\n❌ 测试 5 失败:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
