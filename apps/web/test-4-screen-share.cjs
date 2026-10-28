const { checkPrereqs, login, randName, injectFakeDisplayMedia } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 测试 4: 屏幕共享 + E2EE\n');
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');

    const alice = await (await browser.newContext()).newPage();
alice.on('console', m => {
  const t = m.text()
  if (/\[media\]|TRUE E2EE|exporter|E2EE|ChannelSync|MlsCommit|MLS|epoch|\[EV\]|EpochChanged/i.test(t)) console.log('[Alice]', t)
})
    await injectFakeDisplayMedia(alice);
    await login(alice, aliceName);
    console.log('👤 Alice:', aliceName);

    const bob = await (await browser.newContext()).newPage();
bob.on('console', m => {
  const t = m.text()
  if (/\[media\]|TRUE E2EE|exporter|E2EE|ChannelSync|MlsCommit|MLS|epoch|\[EV\]|EpochChanged/i.test(t)) console.log('[Bob]', t)
})
    await login(bob, bobName);
    console.log('👤 Bob:', bobName);

    console.log('⏳ 等待自动拉人...');
    await alice.waitForTimeout(4000);

    console.log('🎤 Alice 加入语音...');
    // 3 次重试（前序测试可能污染 LiveKit 房间状态）
    // 成功标志 = media-leave 出现（MainView 里 media.connected 后切换）
    //   或 media-connected 出现（MediaBar 里的连接状态），两者之一即可
    const MEDIA_OK_SEL = '[data-testid="media-leave"], [data-testid="media-connected"]';
    for (let i = 0; i < 3; i++) {
      // 幂等：已连接就跳过
      if (await alice.locator(MEDIA_OK_SEL).count() > 0) {
        console.log('   已是连接状态，跳过');
        break;
      }
      await alice.click('[data-testid="channel-item-voice"]');
      await alice.waitForTimeout(3000);
      // 只有按钮是 media-join 才点（防重入）
      if (await alice.locator('[data-testid="media-join"]').count() > 0) {
        await alice.click('[data-testid="media-join"]');
        await alice.waitForTimeout(3000);
      }
      try {
        await alice.waitForSelector(MEDIA_OK_SEL, { timeout: 15000 });
        break;
      } catch (e) {
        if (i === 2) throw e;
        console.log(`   加入语音第 ${i + 1} 次失败，重试...`);
        await alice.waitForTimeout(2000);
      }
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
      try {
        await bob.waitForSelector(MEDIA_OK_SEL, { timeout: 15000 });
        break;
      } catch (e) {
        if (i === 2) throw e;
        await bob.waitForTimeout(2000);
      }
    }
    await alice.waitForTimeout(2000);

    console.log('🖥️  Alice 用 720p30 共享...');
    // compact MediaBar 可能没有 quality select（画质切换是 test-6 的职责）
    // 存在才选，不存在跳过
    if (await alice.locator('[data-testid="media-quality"]').count() > 0) {
      await alice.locator('[data-testid="media-quality"]').selectOption('720p30');
      console.log('   画质已设为 720p30');
    } else {
      console.log('   compact MediaBar 无画质选择器，跳过');
    }
    await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });
    await alice.click('[data-testid="media-share-toggle"]');

    console.log('📺 Bob 等待画面...');
    await bob.waitForFunction(() => {
      for (const v of document.querySelectorAll('video')) {
        if (v.videoWidth > 0) return true;
      }
      return false;
    }, { timeout: 20000 });

    const info = await bob.evaluate(() => {
      for (const v of document.querySelectorAll('video')) {
        if (v.videoWidth > 0) return { w: v.videoWidth, h: v.videoHeight };
      }
      return null;
    });
    console.log('📊 分辨率:', JSON.stringify(info));
    if (!info || info.w < 640) {
      console.log('❌ 分辨率异常');
      process.exit(1);
    }

    console.log('🔐 检查 E2EE...');
    const aliceE2EE = await alice.locator('[data-testid="media-e2ee"][data-e2ee="true"]').count();
    const bobE2EE = await bob.locator('[data-testid="media-e2ee"][data-e2ee="true"]').count();
    console.log(`   Alice: ${aliceE2EE > 0 ? '✅' : '❌'}, Bob: ${bobE2EE > 0 ? '✅' : '❌'}`);
    if (aliceE2EE === 0 || bobE2EE === 0) {
      console.log('❌ E2EE 未启用');
      process.exit(1);
    }

    console.log('🛑 Alice 停止共享...');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });

    await bob.waitForFunction(() => {
      for (const v of document.querySelectorAll('video')) {
        if (v.videoWidth > 0) return false;
      }
      return true;
    }, { timeout: 10000 });
    console.log('✅ Bob 端画面已消失');

    console.log('\n🎉 测试 4 通过');
  } catch (e) {
    console.error('\n❌ 测试 4 失败:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
