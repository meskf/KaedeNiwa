import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel, myIdentity,
  installSyntheticDisplay, sampleStreamQuality, summarize, sampleFramePacing,
} from '../lib/fixtures';

/**
 * 屏幕共享帧率/连续性回归（对应线上反馈：“读文档还好，一打游戏就掉帧、断续”）。
 *
 * 这里的帧源不是 headless 的假桌面，而是 canvas.captureStream 的**每帧都在变化**
 * 的真实视频源，所以 framesEncoded / framesDecoded 是有效指标。
 *
 * 判定口径：
 *   - avgFps：接收端每秒实际解码帧数均值（断续会直接把它拉低）
 *   - starved：某一秒帧数 < 均值 1/3 的“空档”次数（这就是肉眼看到的“一卡一卡”）
 *   - frozenMs：WebRTC freezeCount/totalFreezesDuration 累计冻结时长
 *   - lossPct：接收端丢包率
 */

async function selectPreset(page: any, id: string) {
  await page.getByTestId('media-quality').selectOption(id);
}

async function startShare(page: any, presetId: string) {
  await selectPreset(page, presetId);
  await page.getByTestId('media-share-toggle').click();
  await expect(page.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'on', { timeout: 30_000 });
}

test.describe('SCREEN SHARE — 帧率与连续性', () => {
  test('1080p30 高动态：接收端平均帧率达标、无连续空档/冻结', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    // 源按 60fps 出帧，由产品自己按 30fps 档位编码 —— 模拟高动态游戏画面
    await installSyntheticDisplay(u0, { width: 1920, height: 1080, fps: 60, motion: 'high' });
    const u0Id = await myIdentity(u0);
    await startShare(u0, '1080p30');

    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
    await expect(u1.getByTestId('screen-share-video').first()).toBeVisible();

    // 预热：等首帧/关键帧/码率收敛，不计入统计
    await u0.waitForTimeout(8_000);

    const recv = summarize(await sampleStreamQuality(u1, u0Id, 15));
    const send = summarize(await sampleStreamQuality(u0, null, 8));
    console.log('[QC][1080p30] receiver', JSON.stringify(recv));
    console.log('[QC][1080p30] sender  ', JSON.stringify(send));

    expect(recv.n, '采样桶数量').toBeGreaterThanOrEqual(10);
    // 30fps 档位允许编码器/调度留一点余量，但低于 20fps 就是肉眼可见的卡
    expect(recv.avgFps, '接收端平均帧率').toBeGreaterThanOrEqual(20);
    expect(send.avgFps, '发送端平均帧率').toBeGreaterThanOrEqual(20);
    expect(recv.starved, '接收端空档秒数').toBeLessThanOrEqual(3);
    expect(recv.frozenMs, '累计冻结时长(ms)').toBeLessThanOrEqual(1500);
    expect(recv.lossPct, '接收端丢包率(%)').toBeLessThanOrEqual(5);
    expect(send.limits, '发送端不应长期受 CPU 限制').not.toContain('cpu');
  });

  test('帧稳定性：1080p30 高动态帧间隔均匀，不靠降帧率换流畅', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await installSyntheticDisplay(u0, { width: 1920, height: 1080, fps: 60, motion: 'high' });
    const u0Id = await myIdentity(u0);
    await startShare(u0, '1080p30');
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    // 预热：关键帧/码率收敛后再测节奏
    await u0.waitForTimeout(8_000);

    const pacing = await sampleFramePacing(u1, u0Id, 15);
    console.log('[QC][pacing-1080p30]', JSON.stringify(pacing));

    // 判定口径是“节奏”而不是“总量”：30fps 档理论帧间隔 33.3ms
    expect(pacing.available, 'Chrome 应提供 totalInterFrameDelay').toBe(true);
    expect(pacing.frames, '窗口内解码帧数').toBeGreaterThan(300);
    expect(Math.abs(pacing.meanMs - 33.3), '平均帧间隔偏离 33.3ms').toBeLessThan(6);
    // 标准差 < 18ms ≈ 半个帧周期：超过就是肉眼可感的忽快忽慢
    expect(pacing.stdMs, '帧间隔标准差(ms)').toBeLessThan(18);
    expect(pacing.dropped, '窗口内丢帧数').toBeLessThanOrEqual(2);
  });

  test('720p60 游戏场景：高动态 60fps 仍保持连续', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    // 60fps 档位 + 720p 源：贴近“游戏共享”的编码压力
    await installSyntheticDisplay(u0, { width: 1280, height: 720, fps: 60, motion: 'high' });
    const u0Id = await myIdentity(u0);
    await startShare(u0, '1080p60');

    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
    await u0.waitForTimeout(8_000);

    const recv = summarize(await sampleStreamQuality(u1, u0Id, 15));
    console.log('[QC][720p60] receiver', JSON.stringify(recv));

    expect(recv.n, '采样桶数量').toBeGreaterThanOrEqual(10);
    expect(recv.avgFps, '接收端平均帧率').toBeGreaterThanOrEqual(35);
    expect(recv.starved, '接收端空档秒数').toBeLessThanOrEqual(3);
    expect(recv.frozenMs, '累计冻结时长(ms)').toBeLessThanOrEqual(1500);
    expect(recv.lossPct, '接收端丢包率(%)').toBeLessThanOrEqual(5);
  });

  test('诊断可用：健康度能识别为 UDP 直连且给出实时帧率', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await installSyntheticDisplay(u0, { width: 1280, height: 720, fps: 30, motion: 'high' });
    await startShare(u0, '720p30');
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    u0.on('pageerror', (e) => console.log('[u0 pageerror]', e.message));
    u1.on('pageerror', (e) => console.log('[u1 pageerror]', e.message));

    // 采样器 1s 一次，等 10s 让窗口稳定
    await u1.waitForTimeout(10_000);

    // 关键：不能只看“已订阅”，必须确认视频元素真的有帧。
    // 之前 E2EE Worker 404 时，订阅状态是 true 但接收端一帧都收不到（假绿）。
    const videoInfo = await u1.evaluate(() => {
      const v = document.querySelector('[data-testid="screen-share-video"]') as HTMLVideoElement | null;
      if (!v) return null;
      const q: any = (v as any).getVideoPlaybackQuality?.() ?? {};
      return { w: v.videoWidth, h: v.videoHeight, readyState: v.readyState, frames: q.totalVideoFrames };
    });
    console.log('[QC][u1 video]', JSON.stringify(videoInfo));
    expect(videoInfo?.w ?? 0, '接收端视频宽度').toBeGreaterThan(0);
    expect(videoInfo?.frames ?? 0, '接收端已解码帧数').toBeGreaterThan(0);

    const health = await u1.evaluate(() => (window as any).__kaedeMediaHealth?.() ?? null);
    console.log('[QC][health]', JSON.stringify(health));
    expect(health, '接收端健康度快照').not.toBeNull();
    expect(health.transport, '媒体传输方式').toBe('udp');
    expect(health.inbound?.fps ?? 0, '接收端帧率').toBeGreaterThan(0);

    // UI 上的健康度徽标也应可见（观众能直接看到“TCP 回落/丢包”）
    await expect(u1.getByTestId('screen-share-health').first()).toBeVisible();
    // 现场没法边玩边敲控制台，必须有“一键导出诊断”
    await expect(u1.getByTestId('screen-share-diag').first()).toBeVisible();
  });

  test('静态文档画面不会误触发自动降档', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await installSyntheticDisplay(u0, { width: 1920, height: 1080, fps: 30, motion: 'static' });
    await startShare(u0, '1080p30');
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    // 静态画面 framesEncoded 天然很低，如果拿“帧率”当降档依据就会误伤
    await u0.waitForTimeout(15_000);
    const state = await u0.evaluate(() => {
      const fn = (window as any).__kaedeMediaState;
      return fn ? fn().autoQualityNote : null;
    });
    expect(state, '静态共享不应触发自动降档').toBeNull();
  });

  test('诊断快照可事后导出：历史/关键帧/事件结构完整', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await installSyntheticDisplay(u0, { width: 1280, height: 720, fps: 30, motion: 'high' });
    await startShare(u0, '720p30');
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    await u0.waitForTimeout(12_000);

    // 现场用户没法边打游戏边在控制台敲命令，所以必须有“事后一键导出”的快照。
    const sender = await u0.evaluate(() => (window as any).__kaedeDiag?.() ?? null);
    expect(sender, '共享端诊断快照').not.toBeNull();
    expect(sender.history.length, '逐秒历史条数').toBeGreaterThanOrEqual(6);
    expect(sender.presetId).toBe('720p30');
    expect(sender.codec, '记录实际使用的编码器').toMatch(/h264|vp8/);
    const lastOut = sender.history[sender.history.length - 1].outbound;
    expect(lastOut?.keyFrames, '出站关键帧计数').toBeGreaterThan(0);

    const viewer = await u1.evaluate(() => (window as any).__kaedeDiag?.() ?? null);
    expect(viewer, '观众端诊断快照').not.toBeNull();
    const lastIn = viewer.history[viewer.history.length - 1].inbound;
    expect(lastIn?.keyFrames, '入站已解关键帧计数').toBeGreaterThan(0);
    // 正常连续共享时不应被判成“有码率无画面”
    expect(lastIn?.stalledSec ?? 0, '稳态不应处于无帧状态').toBe(0);
    expect(Array.isArray(viewer.events), '事件列表').toBe(true);
  });

  test('手机端“横屏放大”会进入全屏', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await installSyntheticDisplay(u0, { width: 1280, height: 720, fps: 30, motion: 'high' });
    await startShare(u0, '720p30');
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    const btn = u1.getByTestId('screen-share-landscape').first();
    await btn.click();
    await expect
      .poll(async () => u1.evaluate(() => !!document.fullscreenElement), { timeout: 10_000 })
      .toBe(true);

    // 再点视频退出全屏，方向锁必须被释放（否则 App 会卡死在横屏）
    await u1.getByTestId('screen-share-video').first().click();
    await expect
      .poll(async () => u1.evaluate(() => !!document.fullscreenElement), { timeout: 10_000 })
      .toBe(false);
  });
});
