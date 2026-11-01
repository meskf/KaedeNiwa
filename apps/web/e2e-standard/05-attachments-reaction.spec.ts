import { test, expect, waitDebug, getFirstTextChannelId } from './fixtures';

test.describe('ATTACHMENTS & REACTION', () => {
  test('发送附件与表情回应', async ({ u0, u1 }) => {
    await waitDebug(u0);
    await waitDebug(u1);
    await u0.waitForTimeout(2000);

    const chId = await getFirstTextChannelId(u0);
    expect(chId).toBeTruthy();

    // 1. 上传附件
    const bytes = new Uint8Array([72, 101, 108, 108, 111]);
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

    // 3. u1 能看到消息
    await expect(u1.getByText(msgText, { exact: false }).first()).toBeVisible({ timeout: 10000 });

    // 4. u0 取目标消息 id/ts，按 id 加表情（v8.1：id 优先）
    const history = await u0.evaluate((ch) => (window as any).__kaede_debug__.loadHistory(ch, 10), chId);
    const targetMsg = history.find((m: any) => JSON.stringify(m).includes(msgText));
    expect(targetMsg).toBeTruthy();
    const msgId = Number(targetMsg.id) || 0;
    const msgTs = targetMsg.ts;
    expect(msgId).toBeGreaterThan(0);

    await u0.evaluate(({ ch, id, ts }) => (window as any).__kaede_debug__.reaction.sendById(ch, id, ts, '👍'), { ch: chId, id: msgId, ts: msgTs });
    await u0.waitForTimeout(1500);

    // 5. u1 能看到 reaction pill
    await expect(u1.locator('[data-testid="reaction-pill-👍"]').first()).toBeVisible({ timeout: 5000 });
  });
});
