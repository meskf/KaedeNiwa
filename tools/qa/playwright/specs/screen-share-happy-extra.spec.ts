import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel,
  myIdentity, videoFrameStats, remoteShareInfo,
} from '../lib/fixtures';

test.describe('SCREEN SHARE — happy path 扩展', () => {
  test('接收端 track 端到端贯通：LiveKit isSubscribed + <video> srcObject 是 live MediaStream', async ({ u0, u1 }) => {
    // ⚠️ 环境边界说明（重要）：
    // Playwright 用 `--use-fake-device-for-media-stream` 起浏览器。在这种环境下：
    //   - sender 发布 / SFU 转发 / receiver 订阅 全部是真实的（LiveKit 侧 isSubscribed=true）
    //   - 但假源在接收端的 MediaStreamTrack 处于 muted=true，不产出真实编码帧
    //   - 因此 video.videoWidth 和 track.getSettings().width 都是 0 / undefined
    //
    // 这三条断言在 fake 源下**不可能成立**，也不能用它们证明"产品坏了"——
    // 它们需要真实桌面源或 Xvfb 里放真实内容才能成立（那是另一层测试，plan 在 L5/L6）。
    //
    // 本 spec 断言"track 已经从 server 转发到接收端并 attach 到 <video> 上"：
    //   1. LiveKit publication.isSubscribed === true   ← server 已把 track 转发过来
    //   2. <video>.srcObject 是 MediaStream 且含 video track   ← 产品把它 attach 到了 DOM
    //   3. track.readyState === 'live' 且 enabled === true   ← track 处于活状态（不是 closed）
    // 这是 fake 源下最强、最稳定的可重复证据。
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);
    const u0Id = await myIdentity(u0);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    // 1. LiveKit 侧：Server 已经把这个 track 转发到 u1，u1 已订阅
    await expect.poll(async () => {
      const r = await remoteShareInfo(u1, u0Id);
      return r.has && r.isSubscribed ? 1 : 0;
    }, { timeout: 20_000, intervals: [500] }).toBe(1);

    // 2. DOM 侧：<video>.srcObject 是 MediaStream 且含 video track
    await expect.poll(async () => {
      return await u1.evaluate(() => {
        const v = document.querySelector('[data-testid="screen-share-video"]') as HTMLVideoElement | null;
        const s = v?.srcObject as MediaStream | null;
        if (!s) return 0;
        return s.getVideoTracks().length;
      });
    }, { timeout: 20_000, intervals: [500] }).toBeGreaterThan(0);

    const probe = await u1.evaluate(() => {
      const v = document.querySelector('[data-testid="screen-share-video"]') as HTMLVideoElement;
      const s = v.srcObject as MediaStream;
      const t = s.getVideoTracks()[0];
      const set = t.getSettings();
      return {
        videoTagFound: !!v,
        srcObjectIsMediaStream: s instanceof MediaStream,
        videoTrackCount: s.getVideoTracks().length,
        kind: t.kind,
        trackReadyState: t.readyState,
        trackEnabled: t.enabled,
        // 以下为诊断信息，fake 源下预期：muted=true / width=undefined / readyState=0
        trackMuted: t.muted,
        videoPaused: v.paused,
        videoReadyState: v.readyState,
        videoWidth: v.videoWidth,
        videoHeight: v.videoHeight,
        settings: set,
        label: t.label,
      };
    });
    console.log('PROBE=' + JSON.stringify(probe));

    // 3. track 处于活状态
    expect(probe.videoTagFound, JSON.stringify(probe)).toBe(true);
    expect(probe.srcObjectIsMediaStream).toBe(true);
    expect(probe.videoTrackCount).toBeGreaterThan(0);
    expect(probe.kind).toBe('video');
    expect(probe.trackReadyState).toBe('live');
    expect(probe.trackEnabled).toBe(true);

    // 诊断输出（不作为断言）
    console.log('SENDER_STATS=' + JSON.stringify(await videoFrameStats(u0, null)));
    console.log('RECEIVER_STATS=' + JSON.stringify(await videoFrameStats(u1, u0Id)));
  });

  test('UI 点"停止共享" -> 双端清理', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    await u0.getByTestId('media-share-toggle').click();
    await expect(u0.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off', { timeout: 20_000 });
    await expect(u0.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 15_000 });
    await expect(u1.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 25_000 });
  });

  test('product onended 处理：dispatchEvent(ended) 触发产品 stopScreenShare，UI 跟随 media state 复位', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.evaluate(() => {
      const orig = navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);
      (navigator.mediaDevices as any).getDisplayMedia = async (c: any) => {
        const s = await orig(c);
        (window as any).__kaede_last_display_track__ = s.getVideoTracks()[0];
        return s;
      };
    });

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    await u0.evaluate(() => {
      const t = (window as any).__kaede_last_display_track__;
      if (!t) throw new Error('no captured track');
      t.dispatchEvent(new Event('ended'));
    });

    await expect(u0.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off', { timeout: 30_000 });
    await expect(u1.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 30_000 });
  });

  test('共享方关闭页面 -> u1 panel 由 ParticipantDisconnected 清理', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    await u0.close();

    await expect(u1.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 60_000 });
  });
});
