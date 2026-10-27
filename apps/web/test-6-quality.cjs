const { checkPrereqs, login, randName, injectFakeDisplayMedia, setDisableAdaptive } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

async function shareAndMeasure(alice, bob, presetId, expectW, expectH, tolerance = 0.15) {
  console.log(`\n📺 测试画质: ${presetId} (期望 ${expectW}x${expectH})`);
  await alice.locator('[data-testid="media-quality"]').selectOption(presetId);
  await alice.click('[data-testid="media-share-toggle"]');
  await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });

  // 等 Bob 收到画面
  await bob.waitForFunction(() => {
    for (const v of document.querySelectorAll('video')) {
      if (v.videoWidth > 0) return true;
    }
    return false;
  }, { timeout: 20000 });

  await bob.waitForTimeout(1000);
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

  // 停止共享（无论成败，避免 select 保持 disabled）
  await alice.click('[data-testid="media-share-toggle"]');
  await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });
  await bob.waitForFunction(() => {
    for (const v of document.querySelectorAll('video')) {
      if (v.videoWidth > 0) return false;
    }
    return true;
  }, { timeout: 10000 });
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

    // 检查下拉框选项
    console.log('\n📋 画质下拉选项:');
    const options = await alice.locator('[data-testid="media-quality"] option').allTextContents();
    options.forEach(o => console.log(`   ${o}`));
    if (options.length < 5) {
      console.log('❌ 画质档位少于 5 个');
      process.exit(1);
    }

    // 依次测试 3 档画质
    const results = [];
    results.push(await shareAndMeasure(alice, bob, '480p30', 854, 480));
    results.push(await shareAndMeasure(alice, bob, '1080p60', 1920, 1080));
    results.push(await shareAndMeasure(alice, bob, '1440p60', 2560, 1440));

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
