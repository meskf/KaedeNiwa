import { test, expect } from '@playwright/test';
import { spawnHumans, closeHumans, openView, waitForWorkspace, humanPause, hardErrors } from '../lib/core';
import {
  getFirstVoiceChannel,
  enterVoiceChannel,
  leaveVoiceChannelViaUI,
  installSyntheticDisplay,
  installGdmNotAllowed,
  startShareViaUI,
  stopShareViaUI,
  waitRemoteShareWithFrames,
  stopDisplayStreamViaBrowser,
  localShareInfo,
  remoteShareInfo,
  myIdentity,
  videoFrameStats,
  getDiag,
  sharePanelCount,
} from '../lib/media';

test.describe('屏幕共享异常与恢复（重点）', () => {
  test('用户取消系统拾取器：不共享、不静默假成功、给出错误提示', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-cancel', {
      beforeLogin: page => installGdmNotAllowed(page),
    });
    try {
      const [h0] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);

      await h0.page.getByTestId('media-share-toggle').click();
      await expect(h0.page.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off');
      await expect(h0.page.getByText(/NotAllowedError|user cancelled|取消/i).first()).toBeVisible({
        timeout: 20_000,
      });
      expect(await localShareInfo(h0.page)).toMatchObject({ has: false });
      expect(await sharePanelCount(h0.page)).toBe(0);

      expect(hardErrors(h0)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('选择 4K 但拾取器拒绝：自动降级 1080p60 并真的出帧', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-4k');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      // 浏览器只支持到 1920 宽
      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high', maxWidth: 1920 });
      await startShareViaUI(h0.page, '4k60');

      const local = await localShareInfo(h0.page);
      expect(local.has).toBe(true);
      expect(local.settings?.width).toBe(1920);
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      const diag = await getDiag(h0.page);
      expect(diag?.presetId).toBe('1080p60');

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('观众点浏览器原生“停止共享”：产品自己收尾，观众画面消失', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-native-stop');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      await stopDisplayStreamViaBrowser(h0.page);

      await expect(h0.page.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off', {
        timeout: 25_000,
      });
      await expect(h1.page.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 25_000 });
      await expect
        .poll(async () => (await localShareInfo(h0.page)).has, { timeout: 20_000, intervals: [500, 1000] })
        .toBe(false);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('真人手快连点共享按钮：最多一路发布，UI 状态与实际发布一致', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-dblclick');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      const toggle = h0.page.getByTestId('media-share-toggle');
      await toggle.click();
      await h0.page.waitForTimeout(150);
      await toggle.click();
      await h0.page.waitForTimeout(5000);

      const pubs = await h0.page.evaluate(() =>
        Array.from((window as any).__kaedeRoom.localParticipant.videoTrackPublications.values())
          .filter((p: any) => p.source === 'screen_share')
          .length,
      );
      expect(pubs).toBeLessThanOrEqual(1);
      const local = await localShareInfo(h0.page);
      const attr = await toggle.getAttribute('data-sharing');
      expect(attr).toBe(local.has ? 'on' : 'off');

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('观众中途加入：共享已开始后才进频道，仍能收到并出帧', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-midjoin');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      const id0 = await myIdentity(h0.page);

      await humanPause(h1.page, 1500, 2500);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('共享中观众切走视图再切回：订阅不断、画面继续', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-view-switch');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      await openView(h1.page, 'friends');
      await h1.page.waitForTimeout(3000);
      await openView(h1.page, 'chat');
      await expect(h1.page.getByTestId('screenshare-panel')).toBeVisible({ timeout: 20_000 });
      const frames = await videoFrameStats(h1.page, id0);
      expect(frames.framesDecoded).toBeGreaterThan(0);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('共享中主播离开语音：发布随之清理，观众面板消失且仍在频道内', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-leave');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      await leaveVoiceChannelViaUI(h0.page, vc.id);
      await expect(h1.page.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 25_000 });
      await expect(h1.page.getByTestId('media-connected')).toHaveAttribute('data-channel', vc.id);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('共享中主播刷新页面：旧共享清理干净，重新进入后可以再次共享', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-reload');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const oldId = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, oldId, 30_000);

      await h0.page.reload();
      await waitForWorkspace(h0.page);
      await expect(h1.page.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 30_000 });

      // 真人重新进频道 + 重新共享
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      const newId = await myIdentity(h0.page);
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, newId, 30_000);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('共享中观众刷新页面：重新进频道后自动重新订阅并出帧', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-viewer-reload');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      await h1.page.reload();
      await waitForWorkspace(h1.page);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('停止后立刻重开：新 trackSid 生效，观众恢复出帧', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ss-restart');
    try {
      const [h0, h1] = humans;
      const vc = await getFirstVoiceChannel(h0.page);
      await enterVoiceChannel(h0.page, vc.id, vc.name);
      await enterVoiceChannel(h1.page, vc.id, vc.name);
      const id0 = await myIdentity(h0.page);

      await installSyntheticDisplay(h0.page, { width: 1920, height: 1080, fps: 60, motion: 'high' });
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);
      const sid1 = (await remoteShareInfo(h1.page, id0)).sid;

      await stopShareViaUI(h0.page);
      await expect(h1.page.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 20_000 });

      await h0.page.waitForTimeout(600);
      await startShareViaUI(h0.page, '1080p30');
      await waitRemoteShareWithFrames(h1.page, id0, 30_000);
      const sid2 = (await remoteShareInfo(h1.page, id0)).sid;
      expect(sid2).not.toBe(sid1);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
