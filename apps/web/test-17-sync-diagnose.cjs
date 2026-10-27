const { login, randName, injectFakeDisplayMedia } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');
const fs = require('fs');

(async () => {
  console.log('🕵️‍♂️ 测试 17: 双端消息 & 画面同步诊断 (修复版)\n');
  const browser = await launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');

    const alice = await (await browser.newContext()).newPage();
    await injectFakeDisplayMedia(alice, { withAudio: true });
    await login(alice, aliceName);

    const bob = await (await browser.newContext()).newPage();
    await login(bob, bobName);

    console.log('⏳ 等待 WebSocket 和 MLS 同步...');
    await alice.waitForTimeout(3000);

    console.log('🎤 双方加入语音...');
    for (const page of [alice, bob]) {
      await page.click('[data-testid="channel-item-voice"]');
      await page.waitForTimeout(2000);
      const joinBtn = page.locator('[data-testid="media-join"]');
      if (await joinBtn.isVisible()) {
        await joinBtn.click();
        await page.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
      }
    }
    await alice.waitForTimeout(2000);

    // ====== 消息测试 ======
    console.log('\n💬 测试消息同步...');
    await alice.click('[data-testid="message-input"]');
    await alice.keyboard.type('hello from alice');
    await alice.keyboard.press('Enter');
    
    await bob.waitForTimeout(3000);
    const bobPageText = await bob.evaluate(() => document.body.innerText);
    const bobReceived = bobPageText.includes('hello from alice');
    console.log(`Bob 是否收到 Alice 消息: ${bobReceived ? '✅ 是' : '❌ 否'}`);

    // ====== 屏幕共享测试 ======
    console.log('\n🖥️ 测试屏幕共享同步...');
    await alice.locator('[data-testid="media-quality"]').selectOption('720p30');
    await alice.click('[data-testid="media-share-toggle"]');
    console.log('Alice 开启共享，等待 Bob 端画面渲染...');
    await alice.waitForTimeout(5000);

    const videoInfo = await bob.evaluate(() => {
      for (const v of document.querySelectorAll('video')) {
        if (v.videoWidth > 0) return { w: v.videoWidth, h: v.videoHeight, playing: !v.paused };
      }
      return null;
    });

    if (videoInfo) {
      console.log(`Bob 是否看到画面: ✅ 是 (分辨率 ${videoInfo.w}x${videoInfo.h}, 正在播放: ${videoInfo.playing})`);
    } else {
      console.log('Bob 是否看到画面: ❌ 否 (videoWidth 为 0 或找不到 video)');
    }

    // 截图保存
    fs.mkdirSync('D:\\dev2\\kaedeniwa\\screenshots', { recursive: true });
    await bob.screenshot({ path: 'D:\\dev2\\kaedeniwa\\screenshots\\bob-view.png' });
    console.log('📸 已保存 Bob 视角截图: screenshots\\bob-view.png');

  } catch (e) {
    console.error('❌ 测试异常:', e);
  } finally {
    await browser.close();
  }
})();
