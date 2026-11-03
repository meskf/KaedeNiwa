import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel,
  myIdentity, remoteShareInfo, dumpRoomState,
} from '../lib/fixtures';

test.describe('SCREEN SHARE — 断连/重连', () => {
  test('u1 断网 5s 重连 -> 仍能收到 u0 共享', async ({ u0, u1, u1Ctx }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);
    const u0Id = await myIdentity(u0);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    // u1 断网
    await u1Ctx.setOffline(true);
    await u1.waitForTimeout(5_000);
    await u1Ctx.setOffline(false);

    // 恢复后：LiveKit 自动重连，重新订阅
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 60_000 });
    const r = await remoteShareInfo(u1, u0Id);
    expect(r?.has, JSON.stringify(r)).toBe(true);
    expect(r?.isSubscribed).toBe(true);
  });

  test('u0 共享中 forceDisconnect（WS 抖动）-> u1 的 panel 保持（LiveKit 独立连接）', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    // 只断 WS，不断 LiveKit
    await u0.evaluate(async () => {
      const dbg = (window as any).__kaede_debug__;
      await dbg.forceDisconnect();
    });
    // 等重连稳定
    await u0.waitForTimeout(8_000);

    // u1 panel 应该在
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 20_000 });
  });

  test('u0 共享中点"离开语音" -> u1 panel 清理', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    await u0.getByTestId('media-leave').click();

    await expect(u1.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 30_000 });
  });
});
