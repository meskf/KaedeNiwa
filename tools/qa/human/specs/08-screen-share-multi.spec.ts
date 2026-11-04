import { test, expect } from '@playwright/test';
import {
  spawnHumans,
  closeHumans,
  createNodeViaUI,
  waitForNode,
  nodeId,
  humanPause,
  hardErrors,
} from '../lib/core';
import {
  getFirstVoiceChannel,
  enterVoiceChannel,
  leaveVoiceChannelViaUI,
  installSyntheticDisplay,
  startShareViaUI,
  stopShareViaUI,
  waitRemoteShareWithFrames,
  videoFrameStats,
  remoteShareInfo,
  myIdentity,
} from '../lib/media';

test.describe('屏幕共享多人场景（重点）', () => {
  test('1 个主播 + 3 个观众：四个客户端同时看到画面；一个观众离开不影响其他人', async ({ browser }) => {
    const humans = await spawnHumans(browser, 4, 'ss-4u');
    try {
      const [h0, h1, h2, h3] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      for (const h of humans) await enterVoiceChannel(h.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');

      for (const viewer of [h1, h2, h3]) {
        await waitRemoteShareWithFrames(viewer.page, id0, 35_000);
        const frames = await videoFrameStats(viewer.page, id0);
        expect(frames.framesDecoded).toBeGreaterThan(0);
      }

      // 一个观众离开语音
      await leaveVoiceChannelViaUI(h2.page, vc.id);
      await expect(h2.page.getByTestId('media-connected')).toHaveCount(0, { timeout: 20_000 });

      // 其余观众持续出帧（对比 3 秒前后帧计数）
      for (const viewer of [h1, h3]) {
        const before = await videoFrameStats(viewer.page, id0);
        await viewer.page.waitForTimeout(3000);
        const after = await videoFrameStats(viewer.page, id0);
        expect(after.ok).toBe(true);
        expect(after.framesDecoded ?? 0).toBeGreaterThan(before.framesDecoded ?? 0);
        await expect(viewer.page.getByTestId('screenshare-panel')).toBeVisible();
      }

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
      expect(hardErrors(h3)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('两个主播同时点共享：观众端并排显示 2 路且都出帧；一路停止后另一路不受影响', async ({ browser }) => {
    const humans = await spawnHumans(browser, 4, 'ss-dual');
    try {
      const [h0, h1, h2, h3] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      for (const h of humans) await enterVoiceChannel(h.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);
      const id1 = await myIdentity(h1.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high', maxWidth: 1920 });
      await installSyntheticDisplay(h1.page, { width: 1280, height: 720, fps: 60, motion: 'high' });

      // 两人几乎同时点“屏幕共享”（真人抢着开）
      await Promise.all([
        startShareViaUI(h0.page, '1080p30'),
        startShareViaUI(h1.page, '720p60'),
      ]);

      for (const viewer of [h2, h3]) {
        await expect(viewer.page.getByTestId('screenshare-panel')).toBeVisible({ timeout: 35_000 });
        await expect(viewer.page.getByTestId('screen-share-video')).toHaveCount(2, { timeout: 35_000 });
        await waitRemoteShareWithFrames(viewer.page, id0, 30_000);
        await waitRemoteShareWithFrames(viewer.page, id1, 30_000);
        await expect(viewer.page.getByText(/屏幕共享（2）/)).toBeVisible();
      }

      const sid1 = (await remoteShareInfo(h2.page, id1)).sid;
      await stopShareViaUI(h1.page);
      await expect(h2.page.getByTestId('screen-share-video')).toHaveCount(1, { timeout: 25_000 });
      await expect
        .poll(async () => (await remoteShareInfo(h2.page, id1)).has, { timeout: 20_000, intervals: [500, 1000] })
        .toBe(false);
      expect(sid1).toBeTruthy();

      // 剩下的那一路继续出帧
      const before = await videoFrameStats(h2.page, id0);
      await h2.page.waitForTimeout(3000);
      const after = await videoFrameStats(h2.page, id0);
      expect(after.framesDecoded ?? 0).toBeGreaterThan(before.framesDecoded ?? 0);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
      expect(hardErrors(h2)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('共享进行中第三人也开共享：动态从 1 路变成 2 路，布局自动切换', async ({ browser }) => {
    const humans = await spawnHumans(browser, 3, 'ss-grow');
    try {
      const [h0, h1, h2] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      for (const h of humans) await enterVoiceChannel(h.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);
      const id1 = await myIdentity(h1.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h2.page, id0, 30_000);
      await expect(h2.page.getByTestId('screen-share-video')).toHaveCount(1);

      await installSyntheticDisplay(h1.page, { width: 1280, height: 720, fps: 30, motion: 'high' });
      await startShareViaUI(h1.page, '720p30');
      await expect(h2.page.getByTestId('screen-share-video')).toHaveCount(2, { timeout: 30_000 });
      await waitRemoteShareWithFrames(h2.page, id1, 30_000);

      // h0 停掉，剩 h1 一路，布局回到单面板
      await stopShareViaUI(h0.page);
      await expect(h2.page.getByTestId('screen-share-video')).toHaveCount(1, { timeout: 25_000 });
      await humanPause(h2.page, 1000, 2000);
      const frames = await videoFrameStats(h2.page, id1);
      expect(frames.framesDecoded).toBeGreaterThan(0);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
      expect(hardErrors(h2)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('共享只在同一语音频道可见：另一个语音频道的人看不到画面', async ({ browser }) => {
    const humans = await spawnHumans(browser, 3, 'ss-iso');
    try {
      const [h0, h1, h2] = humans;
      const tag = Date.now().toString(36);
      const other = `iso-voice-${tag}`;
      await createNodeViaUI(h0.page, { name: other, kind: 'voice' });
      for (const h of [h1, h2]) await waitForNode(h.page, other);

      const vc = await getFirstVoiceChannel(h0.page);
      for (const h of [h0, h1]) await enterVoiceChannel(h.page, vc.id, vc.name);
      const otherId = await nodeId(h0.page, other);
      await enterVoiceChannel(h2.page, otherId, other);

      const id0 = await myIdentity(h0.page);
      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      // 另一频道的 h2 在整段时间内都不该出现共享面板
      for (let i = 0; i < 4; i++) {
        await h2.page.waitForTimeout(2000);
        await expect(h2.page.getByTestId('screenshare-panel')).toHaveCount(0);
      }
      // h2 仍在自己的语音频道里（没有被误踢/误连）
      await expect(h2.page.getByTestId('media-connected')).toBeVisible({ timeout: 10_000 });

      await stopShareViaUI(h0.page);
      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
      expect(hardErrors(h2)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
