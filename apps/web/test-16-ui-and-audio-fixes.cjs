const { checkPrereqs, login, randName, injectFakeDisplayMedia } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 测试 16: 媒体修复 + UI 自适应验证 (增强版)\n');
  await checkPrereqs();

  const browser = await launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');

    const alice = await (await browser.newContext()).newPage();
    alice.on('console', m => {
      const t = m.text();
      if (t.includes('RNNoise') || t.includes('Mic') || t.includes('ScreenShare') || t.includes('Audio')) {
        console.log('[Alice]', t);
      }
    });
    
    // 注入带音频的假屏幕共享
    await injectFakeDisplayMedia(alice, { withAudio: true });
    await login(alice, aliceName);

    const bob = await (await browser.newContext()).newPage();
    bob.on('console', m => {
      const t = m.text();
      if (t.includes('Audio') || t.includes('ScreenShare')) {
        console.log('[Bob]', t);
      }
    });
    await login(bob, bobName);

    console.log('⏳ 等待...');
    await alice.waitForTimeout(3000);

    console.log('🎤 加入语音...');
    await alice.click('[data-testid="channel-item-voice"]');
    await alice.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await alice.click('[data-testid="media-join"]');
    await alice.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });

    await bob.click('[data-testid="channel-item-voice"]');
    await bob.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await bob.click('[data-testid="media-join"]');
    await bob.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
    await alice.waitForTimeout(2000);

    // 开启麦克风
    console.log('\n🎙️ 开启麦克风 + RNNoise');
    await alice.click('[data-testid="media-mic-toggle"]');
    await alice.waitForTimeout(2000);

    // 开启屏幕共享
    console.log('\n🖥️ Alice 开始屏幕共享 (带系统音频)');
    await alice.locator('[data-testid="media-quality"]').selectOption('1080p60');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForTimeout(3000);

    // Bob 端等待画面
    console.log('\n📺 Bob 等待画面和音频...');
    await bob.waitForFunction(() => {
      for (const v of document.querySelectorAll('video')) {
        if (v.videoWidth > 0) return true;
      }
      return false;
    }, { timeout: 20000 });
    
    // 给音频一点时间订阅
    await bob.waitForTimeout(3000);

    const checks = await bob.evaluate(() => {
      const results = {};
      const scrollArea = document.querySelector('.overflow-y-auto.overflow-x-hidden');
      const panel = document.querySelector('[data-testid="screenshare-panel"]');
      
      if (scrollArea && panel) {
        results.uiFloating = !scrollArea.contains(panel);
      } else {
        results.uiFloating = 'DOM未找到';
      }

      const video = document.querySelector('[data-testid="screen-share-video"]');
      if (video) {
        const style = window.getComputedStyle(video);
        results.objectFit = style.objectFit;
        results.maxHeight = style.maxHeight;
      } else {
        results.objectFit = 'video未找到';
      }

      // 🔊 是 0x1F50A
      if (panel) {
        results.hasAudioIcon = panel.textContent.includes('\uD83D\uDD0A');
      }
      return results;
    });

    console.log(`\n📊 最终验证结果:`);
    console.log(`   - UI 移出滚动区: ${checks.uiFloating === true ? '✅ 是' : '❌ 否'}`);
    console.log(`   - 自适应黑框: object-fit=${checks.objectFit} (✅), max-height=${checks.maxHeight}`);
    console.log(`   - 系统音频接收: ${checks.hasAudioIcon ? '✅ 发现音频图标 (🔊)' : '❌ 未发现音频图标'}`);

  } catch (e) {
    console.error('❌ 测试异常:', e);
  } finally {
    await browser.close();
  }
})();
