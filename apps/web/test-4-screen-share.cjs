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
    for (let i = 0; i < 3; i++) {
      await alice.click('[data-testid="channel-item-voice"]');
    await alice.waitForTimeout(3000);
    await alice.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await alice.click('[data-testid="media-join"]');
    await alice.waitForTimeout(3000);
      try {
        await alice.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 10000 });
        break;
      } catch (e) {
        if (i === 2) throw e;
        console.log(`   加入语音第 ${i + 1} 次失败，重试...`);
        await alice.waitForTimeout(2000);
      }
    }

    console.log('🎤 Bob 加入语音...');
    await bob.click('[data-testid="channel-item-voice"]');
    await bob.waitForTimeout(3000);
    await bob.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await bob.click('[data-testid="media-join"]');
    await bob.waitForTimeout(3000);
    await bob.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
    await alice.waitForTimeout(2000);

    console.log('🖥️  Alice 用 720p30 共享...');
    await alice.locator('[data-testid="media-quality"]').selectOption('720p30');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });

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
