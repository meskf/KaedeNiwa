import { test, expect, sendMessage } from './fixtures';

test.describe('SMOKE', () => {
  test('u0 登录后主界面可见', async ({ u0 }) => {
    await expect(u0.getByTestId('message-input')).toBeVisible();
  });

  test('u0 发消息，自己可见', async ({ u0 }) => {
    const text = `SMOKE_SELF_${Date.now()}`;
    await sendMessage(u0, text);
    await expect(u0.getByText(text, { exact: false }).first()).toBeVisible({ timeout: 15_000 });
  });

  test('u0 发消息，u1 收到（端到端）', async ({ u0, u1 }) => {
    // 双端登录后默认在同一频道（登录即 join）
    // 若这条不稳定，说明产品层的 join 逻辑本身脆弱——正是需要 Playwright 早发现的那类问题
    const text = `SMOKE_E2E_${Date.now()}`;
    await sendMessage(u0, text);
    await expect(u1.getByText(text, { exact: false }).first()).toBeVisible({ timeout: 25_000 });
  });
});
