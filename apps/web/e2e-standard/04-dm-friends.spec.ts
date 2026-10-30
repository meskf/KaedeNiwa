import { test, expect, waitDebug } from './fixtures';

test.describe('DM & FRIENDS', () => {
  test('完整好友与 DM 链路：加好友 -> 同意 -> 建立会话 -> 发消息 -> 互相解密', async ({ u0, u1, u0Name, u1Name }) => {
    await waitDebug(u0);
    await waitDebug(u1);
    await u0.waitForTimeout(2000);

    // 1. u0 发送好友请求
    await u0.evaluate((target) => (window as any).__kaede_debug__.friends.sendRequest(target), u1Name);
    await u0.waitForTimeout(1000);

    // 2. u1 接受好友请求（可以看下 events 或者直接调用）
    await u1.evaluate((target) => (window as any).__kaede_debug__.friends.accept(target), u0Name);
    await u1.waitForTimeout(1000);

    // 验证双方好友列表
    const u0Friends = await u0.evaluate(() => (window as any).__kaede_debug__.friends.list());
    const u1Friends = await u1.evaluate(() => (window as any).__kaede_debug__.friends.list());
    expect(JSON.stringify(u0Friends)).toContain(u1Name);
    expect(JSON.stringify(u1Friends)).toContain(u0Name);

    // 3. u0 打开与 u1 的 DM
    const conv = await u0.evaluate((peer) => (window as any).__kaede_debug__.dm.open(peer), u1Name);
    expect(conv.conv_id).toBeTruthy();

    // 4. u0 发消息
    const msg = `DM_SECRET_${Date.now()}`;
    await u0.evaluate(({ id, text }) => (window as any).__kaede_debug__.dm.send(id, text), { id: conv.conv_id, text: msg });
    await u0.waitForTimeout(2000);

    // 5. u1 检查历史记录解密
    const u1Hist = await u1.evaluate((id) => (window as any).__kaede_debug__.dm.history(id, 50), conv.conv_id);
    expect(JSON.stringify(u1Hist)).toContain(msg);
  });
});
