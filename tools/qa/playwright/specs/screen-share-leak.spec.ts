import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel,
  localShareInfo, dumpRoomState,
} from '../lib/fixtures';

test.describe('SCREEN SHARE — 资源泄漏', () => {
  test('连续 toggle 10 次后无残留 publication / panel', async ({ u0 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);

    for (let i = 0; i < 10; i++) {
      // 主断言用 data-sharing（用户看到的状态，派生自 media.localScreenSharing）。
      // diag-leak 已确认：data-sharing 与 LiveKit isScreenShareEnabled 同步准确。
      // 不用 localShareInfo 是因为 unpublish 的 async 窗口里可能短暂 has:true，
      // 会把"UI 已 off"误判为"泄漏"。
      await u0.getByTestId('media-share-toggle').click();
      await expect(u0.getByTestId('media-share-toggle'))
        .toHaveAttribute('data-sharing', 'on', { timeout: 25_000 });

      await u0.getByTestId('media-share-toggle').click();
      await expect(u0.getByTestId('media-share-toggle'))
        .toHaveAttribute('data-sharing', 'off', { timeout: 15_000 });

      // 每轮循环之间等 > 产品 startScreenShare 的 1s isStartingShare 节流窗口。
      // 这是产品对"防重复点击"的合理保护，不是 bug。
      await u0.waitForTimeout(1200);
    }

    // 所有轮次完成，等 UI + LiveKit 状态都收敛
    await u0.waitForTimeout(1500);
    await expect(u0.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 15_000 });

    // 无残留 publication
    const state = await dumpRoomState(u0);
    const leftover = (state as any).localVideoPubs?.filter((p: any) => p.source === 'screen_share') ?? [];
    expect(leftover, 'localVideoPubs 里还有 screen_share: ' + JSON.stringify(leftover)).toHaveLength(0);

    // localShareInfo 二次确认（此时 unpublish 已完成，track 状态应已稳定）
    const local = await localShareInfo(u0);
    expect(local?.has, JSON.stringify(local)).toBe(false);

    // 且产品报告 isScreenShareEnabled=false
    const enabled = await u0.evaluate(async () => {
      const m: any = await import('/src/lib/livekit.ts');
      const r = m.getCurrentRoom();
      return r ? !!r.localParticipant.isScreenShareEnabled : null;
    });
    expect(enabled, 'room.localParticipant.isScreenShareEnabled').toBe(false);
  });
});
