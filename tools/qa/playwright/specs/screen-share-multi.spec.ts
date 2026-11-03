import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel,
  myIdentity, remoteShareInfo, login,
} from '../lib/fixtures';

test.describe('SCREEN SHARE — 多人/并发', () => {
  test('u0/u1 同时共享 -> 双方 panel 各展示 2 路 video', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u0.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    await u1.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'on', { timeout: 25_000 });

    // 双方都应该看到 2 路 screen-share-video
    await expect(u0.getByTestId('screen-share-video')).toHaveCount(2, { timeout: 30_000 });
    await expect(u1.getByTestId('screen-share-video')).toHaveCount(2, { timeout: 30_000 });
  });

  test('late joiner：u0 先共享，u1 后进 -> 也能收到', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u0.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    // u1 现在才进
    await enterVoiceChannel(u1, chId, name);

    const u0Id = await myIdentity(u0);
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 40_000 });
    const r = await remoteShareInfo(u1, u0Id);
    expect(r?.has, JSON.stringify(r)).toBe(true);
    expect(r?.isSubscribed).toBe(true);
  });

  test('u0 停止 -> u1 停止 -> 双方 panel 归零', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
    await u1.getByTestId('media-share-toggle').click();
    await expect(u0.getByTestId('screen-share-video')).toHaveCount(2, { timeout: 30_000 });

    await u0.getByTestId('media-share-toggle').click();
    await u1.getByTestId('media-share-toggle').click();

    await expect(u0.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 30_000 });
    await expect(u1.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 30_000 });
  });
});
