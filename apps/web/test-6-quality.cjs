const { checkPrereqs, login, randName, injectFakeDisplayMedia, setDisableAdaptive } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

async function shareAndMeasure(alice, bob, presetId, expectW, expectH, tolerance = 0.15) {
  console.log(`\n📺 测试画质: ${presetId} (期望 ${expectW}x${expectH})`);

  // 1. Alice 切换发送 preset 并开始共享
  await alice.locator('[data-testid="media-quality"]').selectOption(presetId);
  await alice.click('[data-testid="media-share-toggle"]');
  await alice.waitForSelector('[data-testid="media-share-toggle"][data-sharing="on"]', { timeout: 10000 });
  console.log('   Alice 已开始共享');

  // 2. Bob 等到画面
  await bob.waitForSelector('[data-testid="screen-share-video"]', { timeout: 20000 });
  await bob.waitForTimeout(1500);

  // 3. Bob 显式切换到对应画质（模拟真实用户在下拉里选择）
  const selectLoc = bob.locator('[data-testid^="screen-share-quality-"]').first();
  const selectCount = await selectLoc.count();
  if (selectCount > 0) {
    try {
      await selectLoc.selectOption(presetId);
      console.log(`   接收端切到 ${presetId}`);
    } catch (e) {
      console.log(`   接收端 selectOption 失败: ${e.message}`);
    }
    await bob.waitForTimeout(3500);
  } else {
    console.log('   ⚠️ 接收端无 quality select');
  }

  // 4. 读取 videoWidth
  const info = await bob.evaluate(() => {
    for (const v of document.querySelectorAll('video')) {
      if (v.videoWidth > 0) return { w: v.videoWidth, h: v.videoHeight };
    }
    return null;
  });
  console.log(`   实际: ${info?.w}x${info?.h}`);

  const wOk = info && Math.abs(info.w - expectW) / expectW <= tolerance;
  const hOk = info && Math.abs(info.h - expectH) / expectH <= tolerance;
  const matched = wOk && hOk;
  if (matched) {
    console.log('   ✅ 匹配');
  } else {
    console.log(`   ❌ 偏差超过 ${tolerance * 100}%`);
  }

  // 5. 停止共享
  await alice.click('[data-testid="media-share-toggle"]');
  await alice.waitForTimeout(1000);
  return matched;
}

(async () => {
  console.log('🧪 测试 6: 画质切换\n');
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

  try {
    const alice = await (await browser.newContext()).newPage();
    await injectFakeDisplayMedia(alice);
    await setDisableAdaptive(alice);
    await login(alice, randName('alice'));
    const bob = await (await browser.newContext()).newPage();
    await setDisableAdaptive(bob);
    await login(bob, randName('bob'));

    console.log('⏳ 等待自动拉人...');
    await alice.waitForTimeout(4000);

    await alice.click('[data-testid="channel-item-voice"]');
    await alice.waitForTimeout(3000);
    await alice.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await alice.click('[data-testid="media-join"]');
    await alice.waitForTimeout(3000);
    await alice.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
    await bob.click('[data-testid="channel-item-voice"]');
    await bob.waitForTimeout(3000);
    await bob.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await bob.click('[data-testid="media-join"]');
    await bob.waitForTimeout(3000);
    await bob.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
    await alice.waitForTimeout(2000);

    console.log('\n📋 画质下拉选项:');
    const options = await alice.locator('[data-testid="media-quality"] option').allTextContents();
    options.forEach(o => console.log(`   ${o}`));
    if (options.length < 5) {
      console.log('❌ 画质档位少于 5 个');
      process.exit(1);
    }

    // 3 档：480p30 / 1080p60 / 1440p60
    // 期望值基于 getDisplayMedia 的实际输出（fake canvas 上限约 1920x1080）
    const results = [];
    results.push(await shareAndMeasure(alice, bob, '480p30', 854, 480));
    results.push(await shareAndMeasure(alice, bob, '720p30', 1280, 720));
    results.push(await shareAndMeasure(alice, bob, '1080p60', 1920, 1080));

    const passed = results.filter(r => r).length;
    console.log(`\n📊 结果: ${passed}/${results.length} 档通过`);

    if (passed < results.length) {
      console.log('❌ 有档位未通过');
      process.exit(1);
    }

    console.log('\n🎉 测试 6 通过');
  } catch (e) {
    console.error('\n❌ 测试 6 失败:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
