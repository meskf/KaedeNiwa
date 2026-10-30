import { test, expect, waitDebug, getFirstTextChannelId, sendMessage } from './fixtures';

test.describe('ATTACHMENTS & REACTION', () => {
  test('发送附件与表情回应', async ({ u0, u1 }) => {
    await waitDebug(u0);
    await waitDebug(u1);
    await u0.waitForTimeout(2000);

    const chId = await getFirstTextChannelId(u0);
    expect(chId).toBeTruthy();

    // 1. 上传附件 (使用一个小字节数组)
    const bytes = new Uint8Array([72, 101, 108, 108, 111]); // "Hello"
    const att = await u0.evaluate(async ({ ch, bytes }) => {
      const dbg = (window as any).__kaede_debug__;
      return await dbg.attachments.upload(ch, 'test.txt', 'text/plain', bytes);
    }, { ch: chId, bytes: Array.from(bytes) });
    expect(att.id).toBeTruthy();

    // 2. 发送带附件的消息
    const msgText = `MSG_WITH_ATT_${Date.now()}`;
    await u0.evaluate(async ({ ch, text, att }) => {
      const dbg = (window as any).__kaede_debug__;
      await dbg.messages.sendWithAttachments(ch, text, [att]);
    }, { ch: chId, text: msgText, att });
    await u0.waitForTimeout(2000);

    // 3. u1 能在 UI 看到消息文本
    await expect(u1.getByText(msgText, { exact: false }).first()).toBeVisible({ timeout: 10000 });

    // 4. u0 对消息发表情
    // 注意：需要获取消息的时间戳，这里可以用 loadHistory 拿到最新的 ts
    const history = await u0.evaluate((ch) => (window as any).__kaede_debug__.loadHistory(ch, 10), chId);
    const targetMsg = history.find((m: any) => JSON.stringify(m).includes(msgText));
    expect(targetMsg).toBeTruthy();
    const msgTs = targetMsg.ts;

    await u0.evaluate(({ ch, ts }) => (window as any).__kaede_debug__.reaction.send(ch, ts, '👍'), { ch: chId, ts: msgTs });
    await u0.waitForTimeout(1500);

    // 5. u1 应该能收到 reaction（可以通过 UI 看到 👍，或者通过 loadHistory 看到）
    // 这里我们先简单断言 UI 中出现了这个 emoji
    await expect(u1.getByText('👍').first()).toBeVisible({ timeout: 5000 });
  });
});
