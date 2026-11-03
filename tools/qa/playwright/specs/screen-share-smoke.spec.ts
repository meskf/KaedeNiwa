import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel,
  localShareInfo, remoteShareInfo, myIdentity,
} from '../lib/fixtures';

test.describe('SCREEN SHARE — smoke (真人 UI 路径)', () => {
  test('u0 进语音 -> 点共享 -> UI 反馈 + 本地发布 screen_share', async ({ u0 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);

    await expect(u0.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off');
    await expect(u0.getByTestId('screenshare-panel')).toHaveCount(0);

    await u0.getByTestId('media-share-toggle').click();

    await expect(u0.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'on', { timeout: 25_000 });
    await expect(u0.getByTestId('screenshare-panel')).toBeVisible();
    await expect(u0.getByTestId('screen-share-video').first()).toBeVisible();

    const local = await localShareInfo(u0);
    expect(local?.has, JSON.stringify(local)).toBe(true);
    expect(local?.isMuted).toBe(false);
    expect(local?.settings?.width ?? 0).toBeGreaterThan(0);
    expect(local?.settings?.height ?? 0).toBeGreaterThan(0);
  });

  test('u0 共享 -> u1 同频道看到 panel 且订阅到 track', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    const u0Id = await myIdentity(u0);
    expect(u0Id.length).toBeGreaterThan(0);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u0.getByTestId('screenshare-panel')).toBeVisible({ timeout: 25_000 });

    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
    await expect(u1.getByTestId('screen-share-video').first()).toBeVisible();

    const remote = await remoteShareInfo(u1, u0Id);
    expect(remote?.has, JSON.stringify(remote)).toBe(true);
    expect(remote?.isSubscribed).toBe(true);
  });

  test('u0 再点一次 -> data-sharing=off -> 双端 panel 消失', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });

    await u0.getByTestId('media-share-toggle').click();
    await expect(u0.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off', { timeout: 20_000 });
    await expect(u0.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 15_000 });
    await expect(u1.getByTestId('screenshare-panel')).toHaveCount(0, { timeout: 20_000 });
  });
});
