import { test, expect, sendMessage, waitDebug, getFirstTextChannelId } from './fixtures';

test.describe('ENCRYPTION', () => {
  test('双端 MLS epoch 推进 且 key_b64 一致（密码学状态验收）', async ({ u0, u1 }) => {
    await waitDebug(u0);
    await waitDebug(u1);
    await u0.waitForTimeout(3_000);

    const chId0 = await getFirstTextChannelId(u0);
    const chId1 = await getFirstTextChannelId(u1);
    expect(chId0, 'u0 没找到默认文字频道').toBeTruthy();
    expect(chId1, 'u1 没找到默认文字频道').toBeTruthy();
    expect(chId0, '双端默认频道不一致').toBe(chId1);

    const aState = await u0.evaluate((ch) => (window as any).__kaede_debug__.channelState(ch), chId0);
    const bState = await u1.evaluate((ch) => (window as any).__kaede_debug__.channelState(ch), chId1);

    console.log('u0 状态:', aState);
    console.log('u1 状态:', bState);

    expect(aState.has_group, 'u0 MLS 组未建立').toBe(true);
    expect(bState.has_group, 'u1 MLS 组未建立').toBe(true);
    expect(aState.main_ready, 'u0 MLS 未就绪').toBe(true);
    expect(bState.main_ready, 'u1 MLS 未就绪').toBe(true);

    // 两人都在组里，初始 epoch=0，加入后必须推进 > 0
    expect(aState.epoch, 'u0 的 epoch 没有推进').toBeGreaterThan(0);
    expect(aState.epoch, '双端 epoch 不一致').toBe(bState.epoch);

    // key 一致性
    expect(typeof aState.key_b64, 'u0 密钥类型错误').toBe('string');
    expect(aState.key_b64.length, 'u0 密钥太短').toBeGreaterThan(20);
    expect(aState.key_b64, '双端派生 key 不一致').toBe(bState.key_b64);
  });

  test('E2EE 可逆：双方 loadHistory 均可解密出明文 canary', async ({ u0, u1 }) => {
    await waitDebug(u0);
    await waitDebug(u1);
    await u0.waitForTimeout(2_000);

    const chId = await getFirstTextChannelId(u0);
    expect(chId).toBeTruthy();

    const canary = `PLAINTEXT_CANARY_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    await sendMessage(u0, canary);
    await u0.waitForTimeout(2_500);

    const history0 = await u0.evaluate((ch) => (window as any).__kaede_debug__.loadHistory(ch, 50), chId);
    const history1 = await u1.evaluate((ch) => (window as any).__kaede_debug__.loadHistory(ch, 50), chId);

    const raw0 = JSON.stringify(history0);
    const raw1 = JSON.stringify(history1);

    expect(raw0, 'u0 解密自身历史失败').toContain(canary);
    expect(raw1, 'u1 解密远端历史失败').toContain(canary);
  });
});
