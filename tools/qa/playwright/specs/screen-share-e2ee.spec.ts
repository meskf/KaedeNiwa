import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel,
  myIdentity, remoteShareInfo, dumpRoomState,
} from '../lib/fixtures';

// 若产品当前未开启 E2EE，本文件 skip（不算失败），保留为持续观察点
test.describe('SCREEN SHARE — E2EE 叠加', () => {
  test('E2EE 开启时共享仍能到达接收端', async ({ u0, u1 }) => {
    const { id: chId, name } = await getFirstVoiceChannel(u0);
    await enterVoiceChannel(u0, chId, name);
    await enterVoiceChannel(u1, chId, name);

    const st = await dumpRoomState(u0);
    if (!(st as any).e2eeEnabled) {
      test.skip(true, 'E2EE 未启用（本环境 e2eeManager.isE2EEEnabled=false），跳过');
      return;
    }

    const u0Id = await myIdentity(u0);
    await u0.getByTestId('media-share-toggle').click();
    await expect(u1.getByTestId('screenshare-panel')).toBeVisible({ timeout: 40_000 });
    const r = await remoteShareInfo(u1, u0Id);
    expect(r?.has, JSON.stringify(r)).toBe(true);
    expect(r?.isSubscribed).toBe(true);

    // 检查 E2EE 状态在两端
    const st1 = await dumpRoomState(u1);
    expect((st1 as any).e2eeEnabled).toBe(true);
  });
});
