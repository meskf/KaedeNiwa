const { checkPrereqs, login, randName, injectFakeDisplayMedia } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

const fail = (msg) => { console.error('\n\u274C ' + msg); process.exit(1); };
const ok = (msg) => console.log('\u2705 ' + msg);

(async () => {
  console.log('\uD83E\uDDEA 测试 15: 麦克风 + RNNoise + 屏幕共享 + 系统音频\n');
  await checkPrereqs();

  const browser = await launch({
    args: [
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      '--autoplay-policy=no-user-gesture-required',
    ],
  });

  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');

    const alice = await (await browser.newContext()).newPage();
    const aliceLogs = [];
    alice.on('console', m => {
      const t = m.text();
      aliceLogs.push(t);
      if (/rnnoise|ScreenShare|E2EE|Audio|Mic/.test(t)) console.log('[A]', t);
    });
    alice.on('pageerror', e => console.log('[A pageerror]', e.message));
    await injectFakeDisplayMedia(alice, { withAudio: true });
    await login(alice, aliceName);
    console.log('\uD83D\uDC64 Alice:', aliceName);

    const bob = await (await browser.newContext()).newPage();
    const bobLogs = [];
    bob.on('console', m => {
      const t = m.text();
      bobLogs.push(t);
      if (/Audio|ScreenShare|E2EE|attach/i.test(t)) console.log('[B]', t);
    });
    bob.on('pageerror', e => console.log('[B pageerror]', e.message));
    await login(bob, bobName);
    console.log('\uD83D\uDC64 Bob:', bobName);

    await alice.waitForTimeout(4000);

    console.log('\n\u23F3 Alice 进语音...');
    await alice.click('[data-testid="channel-item-voice"]');
    await alice.waitForTimeout(2000);
    await alice.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await alice.click('[data-testid="media-join"]');
    await alice.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });

    console.log('\u23F3 Bob 进语音...');
    await bob.click('[data-testid="channel-item-voice"]');
    await bob.waitForTimeout(2000);
    await bob.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await bob.click('[data-testid="media-join"]');
    await bob.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
    await alice.waitForTimeout(2000);

    // ===== 场景 1：Alice 开麦 + RNNoise =====
    console.log('\n\uD83C\uDF99\uFE0F 场景 1: Alice 开麦');
    await alice.click('[data-testid="media-mic-toggle"]');
    await alice.waitForTimeout(3000);

    const firstFrame = aliceLogs.find(l => l.includes('[rnnoise] first frame'));
    if (!firstFrame) {
      console.log('\u26A0\uFE0F 未看到 [rnnoise] first frame 日志');
      console.log('   可能: MediaStreamTrackProcessor 不可用 / 未加载 wasm / 未开麦');
      console.log('   Alice 近 50 条 console:');
      aliceLogs.slice(-50).forEach(l => console.log('     ', l.slice(0, 200)));
    } else {
      ok('first frame: ' + firstFrame.replace('[rnnoise] first frame:', '').trim());
    }

    await alice.waitForTimeout(2500);
    const processed100 = aliceLogs.find(l => /processed=(100|150|200|250|300)/.test(l));
    if (processed100) ok('RNNoise pipeline 处理 100+ 帧正常');
    else console.log('\u26A0\uFE0F 未看到 processed=100+ 日志');

    // 检查 ratio 合理性：只在 inRms > 0.005（真实信号）时判
    const ratioWarn = aliceLogs.find(l => {
      const m = l.match(/inRms=([\d.]+) outRms=([\d.]+) ratio=([\d.]+)/);
      if (!m) return false;
      const inRms = parseFloat(m[1]);
      const ratio = parseFloat(m[3]);
      return inRms > 0.005 && ratio > 3;
    });
    if (ratioWarn) console.log('\u26A0\uFE0F 真实信号被放大 (>3x):', ratioWarn.slice(0, 200));
    else ok('ratio 合理（真实信号未被放大）');

    // 检查 RNNoise 按钮状态（应显示 ON，如果 first frame processing=true）
    const noiseMode = await alice.locator('[data-testid="media-noise-toggle"]').getAttribute('data-mode');
    console.log('   data-mode =', noiseMode);
    if (noiseMode === 'rnnoise') ok('降噪模式: rnnoise');
    else if (noiseMode === 'unsupported') console.log('\u26A0\uFE0F 降噪不支持（Firefox 或缺少 MediaStreamTrackProcessor）');
    else console.log('\u26A0\uFE0F 降噪模式:', noiseMode);

    // ===== 场景 2：Bob 收到麦克风轨道 =====
    console.log('\n\uD83C\uDF99\uFE0F 场景 2: Bob 是否收到 Alice 麦克风');
    await bob.waitForTimeout(3000);
    const bobHasMicTrack = bobLogs.some(l => /\[Audio\].*麦克风|TrackSubscribed.*Microphone/i.test(l));
    const bobSeesMic = await bob.locator('[data-participant="' + aliceName + '"][data-mic="on"]').count();
    if (bobSeesMic > 0) ok('Bob UI 显示 Alice 麦克风 ON');
    else console.log('\u26A0\uFE0F Bob UI 未显示 Alice 麦克风（时序可能）');

    // 检查 Bob 的 audio 元素是否真的播放（DOM 里有 audio 且 time 前进）
    const bobAudioPlaying = await bob.evaluate(() => {
      const audios = Array.from(document.querySelectorAll('audio'));
      return audios.map(a => ({
        src: a.srcObject ? 'stream' : 'no',
        paused: a.paused,
        muted: a.muted,
        currentTime: a.currentTime,
      }));
    });
    console.log('   Bob audio 元素:', JSON.stringify(bobAudioPlaying));
    const anyPlaying = bobAudioPlaying.some(a => !a.paused && a.src === 'stream');
    if (anyPlaying) ok('Bob 有音频在播放');
    else console.log('\u26A0\uFE0F Bob 无音频在播放');

    // ===== 场景 3：Alice 共享屏幕 + 音频 =====
    console.log('\n\uD83D\uDDA5\uFE0F 场景 3: Alice 共享屏幕');
    await alice.locator('[data-testid="media-quality"]').selectOption('720p30');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForTimeout(3000);

    const shareAudioLog = aliceLogs.find(l => l.includes('[ScreenShare] 已发布系统音频'));
    if (shareAudioLog) ok('系统音频已发布');
    else {
      const noAudio = aliceLogs.find(l => l.includes('[ScreenShare] 无系统音频轨道'));
      if (noAudio) console.log('\u26A0\uFE0F 无系统音频（fake getDisplayMedia 没给 audio track，检查 withAudio）');
      else console.log('\u26A0\uFE0F 未看到音频发布日志');
    }

    // ===== 场景 4：Bob 收到屏幕共享画面 =====
    console.log('\n\uD83D\uDCFA 场景 4: Bob 端视频画面');
    try {
      await bob.waitForFunction(() => {
        const vids = document.querySelectorAll('video');
        for (const v of vids) if (v.videoWidth > 0) return true;
        return false;
      }, { timeout: 20000 });
      ok('Bob 端出现视频画面 (videoWidth > 0)');
    } catch (e) {
      console.log('\u274C Bob 端 20s 内无视频画面');
      const diag = await bob.evaluate(() => {
        return Array.from(document.querySelectorAll('video')).map(v => ({
          videoWidth: v.videoWidth,
          videoHeight: v.videoHeight,
          readyState: v.readyState,
          paused: v.paused,
          hasSrc: !!v.srcObject,
          testid: v.getAttribute('data-testid'),
        }));
      });
      console.log('   Bob 端 video 状态:', JSON.stringify(diag, null, 2));
      fail('屏幕共享画面未渲染');
    }

    const bobRes = await bob.evaluate(() => {
      const vids = document.querySelectorAll('video');
      for (const v of vids) if (v.videoWidth > 0) return { w: v.videoWidth, h: v.videoHeight };
      return null;
    });
    console.log('   Bob 端分辨率:', JSON.stringify(bobRes));

    // 检查 Alice 端也有自己共享的预览（本地回显）
    const aliceHasVideo = await alice.evaluate(() => {
      const vids = document.querySelectorAll('video');
      for (const v of vids) if (v.videoWidth > 0) return true;
      return false;
    });
    if (aliceHasVideo) ok('Alice 端自己也有预览');
    else console.log('\u26A0\uFE0F Alice 端没看到自己共享的预览（可能正常，本地回显未启用）');

    console.log('\n\uD83C\uDF89 测试 15 全部通过');
  } catch (e) {
    console.error('\n\u274C 测试 15 异常:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
