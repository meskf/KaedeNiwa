import { test, expect } from '@playwright/test';
import {
  spawnHumans,
  closeHumans,
  openView,
  humanType,
  humanPause,
  sendMessageHuman,
  hardErrors,
} from '../lib/core';

test.describe('真人社交：好友请求 / 接受 / 私信双向 / 删除好友', () => {
  test('A 加 B → B 接受 → 双方出现在好友列表 → 私信往返 → 删除', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'fr');
    try {
      const [h0, h1] = humans;

      // A 在好友页发起请求
      await openView(h0.page, 'friends');
      await h0.page.getByTestId('friends-add-open').click();
      await h0.page.getByTestId('friends-add-input').click();
      await humanType(h0.page, h1.name, { delay: 20 });
      await h1.page.getByTestId('nav-friends').click(); // B 也切到好友页
      await h0.page.getByTestId('friends-add-submit').click();

      // B 端出现待接受请求（组件挂载时拉取；若请求到达稍晚，重进一次好友页）
      const accept = h1.page.getByTestId(`friend-accept-${h0.name}`);
      try {
        await accept.waitFor({ state: 'visible', timeout: 20_000 });
      } catch {
        await openView(h1.page, 'chat');
        await openView(h1.page, 'friends');
        await accept.waitFor({ state: 'visible', timeout: 20_000 });
      }
      await accept.click();

      // A 重新进入好友页刷新列表
      await openView(h0.page, 'chat');
      await openView(h0.page, 'friends');
      await expect(h0.page.getByTestId(`friend-row-${h1.name}`)).toBeVisible({ timeout: 20_000 });
      await expect(h1.page.getByTestId(`friend-row-${h0.name}`)).toBeVisible({ timeout: 20_000 });

      // A 点“发消息”打开私信并发送
      await h0.page.getByTestId(`friend-message-${h1.name}`).click();
      await expect(h0.page.getByTestId('message-input')).toBeVisible({ timeout: 15_000 });
      const hello = '私信你好-' + Date.now().toString(36);
      await sendMessageHuman(h0.page, hello);
      await expect(h0.page.getByTestId('dm-message').filter({ hasText: hello })).toBeVisible({
        timeout: 20_000,
      });

      // B 进入私信：会话应出现；若未自动出现，用“新对话”按账号打开
      await openView(h1.page, 'dms');
      const conv = h1.page.getByTestId(`dm-conv-${h0.name}`);
      if (!(await conv.isVisible().catch(() => false))) {
        await h1.page.getByTestId('dm-new-toggle').click();
        await h1.page.getByTestId('dm-new-input').click();
        await humanType(h1.page, h0.name, { delay: 20 });
        await h1.page.getByTestId('dm-new-open').click();
      }
      await conv.waitFor({ state: 'visible', timeout: 25_000 });
      await conv.click();
      await expect(h1.page.getByTestId('dm-message').filter({ hasText: hello })).toBeVisible({
        timeout: 30_000,
      });

      // B 回复
      const reply = '收到，私信回复-' + Date.now().toString(36);
      await sendMessageHuman(h1.page, reply);
      await expect(h0.page.getByTestId('dm-message').filter({ hasText: reply })).toBeVisible({
        timeout: 30_000,
      });

      // A 删除好友：回到好友页并点删除
      await openView(h0.page, 'friends');
      await h0.page.getByTestId(`friend-remove-${h1.name}`).click();
      await expect(h0.page.getByTestId(`friend-row-${h1.name}`)).toHaveCount(0, { timeout: 20_000 });

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
