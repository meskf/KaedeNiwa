import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import { spawnHumans, closeHumans, humanPause, hardErrors } from '../lib/core';
import {
  getFirstVoiceChannel,
  enterVoiceChannel,
  installSyntheticDisplay,
  startShareViaUI,
  stopShareViaUI,
  waitRemoteShareWithFrames,
  videoFrameStats,
  sampleStreamQuality,
  summarize,
  sampleFramePacing,
  localShareInfo,
  remoteShareInfo,
  myIdentity,
  getDiag,
  getMediaState,
  sharePanel,
} from '../lib/media';

test.describe('屏幕共享核心链路（重点）', () => {
  test('共享开始 → 观众真实出帧 → 停止 → 面板消失且无残留发布', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-life');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');

      await waitRemoteShareWithFrames(h1.page, id0, 30_000);
      const panel = sharePanel(h1.page);
      await expect(panel).toBeVisible();
      const video = h1.page.getByTestId('screen-share-video').first();
      await expect(video).toBeVisible();

      const local = await localShareInfo(h0.page);
      expect(local.has).toBe(true);
      expect(local.isScreenShareEnabled).toBe(true);
      const remote = await remoteShareInfo(h1.page, id0);
      expect(remote.has).toBe(true);

      // 真人点“停止共享”
      await humanPause(h0.page, 400, 800);
      await stopShareViaUI(h0.page);
      await expect(h0.page.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off');

      await expect(panel).toHaveCount(0, { timeout: 20_000 });
      await expect(h1.page.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 20_000 });
      await expect
        .poll(async () => (await localShareInfo(h0.page)).has, { timeout: 20_000, intervals: [500, 1000] })
        .toBe(false);
      await expect
        .poll(async () => (await remoteShareInfo(h1.page, id0)).has, { timeout: 20_000, intervals: [500, 1000] })
        .toBe(false);

      // 停掉后 demo 画面不应残留在观众端
      expect(await videoFrameStats(h1.page, id0)).toMatchObject({ ok: false });

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('1080p30 高动态画面 12s：接收帧率 / 丢包 / 冻结 / 断流桶真实达标', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-quality');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      const recv = summarize(await sampleStreamQuality(h1.page, id0, 12));
      const send = summarize(await sampleStreamQuality(h0.page, null, 6));
      console.log('[ss-quality] recv', JSON.stringify(recv), 'send', JSON.stringify(send));

      expect(recv.n).toBeGreaterThanOrEqual(8);
      expect(recv.avgFps).toBeGreaterThanOrEqual(18);
      expect(recv.starved).toBeLessThanOrEqual(4);
      expect(recv.lossPct).toBeLessThanOrEqual(8);
      expect(recv.freezes).toBeLessThanOrEqual(4);
      expect(recv.frozenMs).toBeLessThanOrEqual(3500);
      expect(send.avgFps).toBeGreaterThanOrEqual(14);

      const health = await h1.page.evaluate(() => (window as any).__kaedeMediaHealth?.() ?? null);
      expect(health).toBeTruthy();
      expect(health.inbound.fps).toBeGreaterThan(10);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('帧间隔稳定性：平均帧率达标且帧与帧之间不忽快忽慢', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-pacing');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      const pacing = await sampleFramePacing(h1.page, id0, 10);
      console.log('[ss-pacing]', JSON.stringify(pacing));
      expect(pacing.available).toBe(true);
      expect(pacing.frames).toBeGreaterThanOrEqual(60);
      expect(pacing.meanMs).toBeGreaterThan(20);
      expect(pacing.meanMs).toBeLessThan(60);
      expect(pacing.stdMs).toBeLessThan(30);
      expect(pacing.dropped).toBeLessThanOrEqual(20);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('720p60 档位：切档后真实帧率明显提升（≥30fps）', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-720p60');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1280, height: 720, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '720p60');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      const recv = summarize(await sampleStreamQuality(h1.page, id0, 10));
      console.log('[ss-720p60]', JSON.stringify(recv));
      expect(recv.n).toBeGreaterThanOrEqual(6);
      expect(recv.avgFps).toBeGreaterThanOrEqual(30);
      expect(recv.lossPct).toBeLessThanOrEqual(8);

      const diag = await getDiag(h0.page);
      expect(diag?.presetId).toBe('720p60');

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('观众端改接收档位：视频不黑屏、不中断出帧', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-recv-quality');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);
      const remote = await remoteShareInfo(h1.page, id0);

      const select = h1.page.getByTestId(`screen-share-quality-${remote.sid}`);
      await expect(select).toBeVisible({ timeout: 15_000 });
      await select.selectOption('720p30');
      await humanPause(h1.page, 2000, 3000);

      const before = await videoFrameStats(h1.page, id0);
      await h1.page.waitForTimeout(3000);
      const after = await videoFrameStats(h1.page, id0);
      expect(after.ok).toBe(true);
      expect(after.framesDecoded ?? 0).toBeGreaterThan(before.framesDecoded ?? 0);
      await expect(h1.page.getByTestId('screen-share-video').first()).toBeVisible();

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('静态文档共享 12s：不误判卡顿、不触发自动降档', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-static');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 30, motion: 'static' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      await h0.page.waitForTimeout(12_000);
      const state = await getMediaState(h0.page);
      console.log('[ss-static] state', JSON.stringify(state));
      expect(String(state?.autoQualityNote ?? '')).toBe('');
      const diag = await getDiag(h0.page);
      expect(diag?.presetId).toBe('1080p30');

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('诊断按钮：真人点击导出 JSON，内容含编码器/档位/逐秒历史', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-diag');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);
      await h0.page.waitForTimeout(8000);

      const downloadPromise = h0.page.waitForEvent('download', { timeout: 20_000 });
      await h0.page.getByTestId('screen-share-diag').click();
      const download = await downloadPromise;
      const path = await download.path();
      expect(path).toBeTruthy();
      const report = JSON.parse(fs.readFileSync(path!, 'utf8'));

      console.log('[ss-diag]', JSON.stringify({ version: report.version, presetId: report.presetId, codec: report.codec, history: report.history?.length }));
      expect(report.version).toBeTruthy();
      expect(report.presetId).toBe('1080p30');
      expect(report.codec === 'h264' || report.codec === 'vp8').toBe(true);
      expect(report.history.length).toBeGreaterThanOrEqual(5);
      expect(report.history[report.history.length - 1].outbound?.fps ?? 0).toBeGreaterThan(0);
      expect(Array.isArray(report.events)).toBe(true);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('全屏放大 / 退出全屏：真人在观众端切换', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-fs');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      await h1.page.getByTestId('screen-share-fullscreen').first().click();
      await expect
        .poll(async () => !!(await h1.page.evaluate(() => document.fullscreenElement)), { timeout: 10_000 })
        .toBe(true);
      await expect(h1.page.getByTestId('screen-share-video').first()).toBeVisible();

      // 再点一次同一个按钮退出全屏（headless 下 Esc 不是浏览器 UI 动作）
      await h1.page.getByTestId('screen-share-fullscreen').first().click();
      await expect
        .poll(async () => !!(await h1.page.evaluate(() => document.fullscreenElement)), { timeout: 10_000 })
        .toBe(false);
      await expect(h1.page.getByTestId('screenshare-panel')).toBeVisible();

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('强制软件编码逃生门 ?codec=vp8：诊断记录 vp8 且观众正常出帧', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-vp8');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h1.page);
      await enterVoiceChannel(h1.page, vc.id, vc.name);

      // 真人切到带参数的地址；localStorage 也一并写上（两条逃生路径都覆盖）
      await h0.page.evaluate(() => localStorage.setItem('kaede.codec', 'vp8'));
      await h0.page.goto('/?codec=vp8');
      await h0.page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 45_000 });
      await h0.page.waitForFunction(() => !!(window as any).__kaede_debug__, null, { timeout: 25_000 });
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      const id0 = await myIdentity(h0.page);

      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);
      await h0.page.waitForTimeout(4000);

      const diag = await getDiag(h0.page);
      expect(diag?.codec).toBe('vp8');

      const frames = await videoFrameStats(h1.page, id0);
      expect(frames.framesDecoded).toBeGreaterThan(0);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
