import { test, expect } from '@playwright/test';
import {
  spawnHumans,
  closeHumans,
  createNodeViaUI,
  renameNodeViaUI,
  deleteNodeViaUI,
  waitForNode,
  nodeByName,
  clickChannel,
  nodeId,
  waitChannelGroup,
  sendMessageHuman,
  messageRowByText,
  countMessageRows,
  humanType,
  humanPause,
  hardErrors,
} from '../lib/core';

test.describe('真人频道管理：分类/子频道/重命名/删除/消息隔离', () => {
  test('右键新建分类与子频道 → 重命名 → 双方同步 → 消息只留在各自频道', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'ch');
    try {
      const [h0, h1] = humans;
      const folder = 'folder-' + Date.now().toString(36);
      const child = 'child-' + Date.now().toString(36);
      const other = 'other-' + Date.now().toString(36);

      // 真人：侧栏 + → 新建分类
      await createNodeViaUI(h0.page, { name: folder, kind: 'folder' });
      await waitForNode(h0.page, folder);
      await waitForNode(h1.page, folder, { timeout: 35_000 });

      // 右键分类 → 新建文字频道
      await createNodeViaUI(h0.page, { name: child, kind: 'text', parent: folder });
      await waitForNode(h0.page, child);
      await waitForNode(h1.page, child, { timeout: 35_000 });

      const childId = await nodeId(h0.page, child);
      // E2EE 频道：真人各自先打开频道（入组拿密钥），再开始聊天
      for (const h of humans) {
        await clickChannel(h.page, child);
        await expect(h.page.getByRole('heading', { name: `#${child}` })).toBeVisible({ timeout: 20_000 });
        await waitChannelGroup(h.page, childId);
      }
      const inChild = 'child-msg-' + Date.now().toString(36);
      await sendMessageHuman(h0.page, inChild);
      await expect(messageRowByText(h1.page, inChild)).toBeVisible({ timeout: 30_000 });

      // 另一个根频道，发另一条消息
      await createNodeViaUI(h0.page, { name: other, kind: 'text' });
      await waitForNode(h0.page, other);
      await waitForNode(h1.page, other, { timeout: 35_000 });
      const otherId = await nodeId(h0.page, other);
      for (const h of humans) {
        await clickChannel(h.page, other);
        await expect(h.page.getByRole('heading', { name: `#${other}` })).toBeVisible({ timeout: 20_000 });
        await waitChannelGroup(h.page, otherId);
      }
      const inOther = 'other-msg-' + Date.now().toString(36);
      await sendMessageHuman(h0.page, inOther);
      await expect(messageRowByText(h1.page, inOther)).toBeVisible({ timeout: 30_000 });

      // 对端切回 child 只能看到 child 的消息
      await clickChannel(h1.page, child);
      await waitChannelGroup(h1.page, childId);
      await expect(messageRowByText(h1.page, inChild)).toBeVisible({ timeout: 30_000 });
      expect(await countMessageRows(h1.page, inOther)).toBe(0);

      // 重命名 child → renamed
      const renamed = child + '-renamed';
      await renameNodeViaUI(h0.page, child, renamed);
      await waitForNode(h0.page, renamed);
      await waitForNode(h1.page, renamed, { timeout: 30_000 });
      await expect(h1.page.locator(`[data-node-id="${childId}"]`)).toContainText(renamed, { timeout: 25_000 });

      // 删除 renamed（原生 confirm 自动接受）
      await deleteNodeViaUI(h0.page, renamed);
      await expect(nodeByName(h0.page, renamed)).toHaveCount(0, { timeout: 20_000 });
      await expect(nodeByName(h1.page, renamed)).toHaveCount(0, { timeout: 20_000 });

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('私密频道：密码错误被拒 → 正确密码进入 → 只看到该频道消息', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'priv');
    try {
      const [h0, h1] = humans;
      const name = 'priv-' + Date.now().toString(36);
      const secret = 'secret-pass-123';
      const token = '私密消息-' + Date.now().toString(36);

      await createNodeViaUI(h0.page, { name, kind: 'private', password: secret });
      await waitForNode(h0.page, name);
      await waitForNode(h1.page, name, { timeout: 35_000 });

      const privId = await nodeId(h0.page, name);

      // 对端先输错密码
      await nodeByName(h1.page, name).click();
      await expect(h1.page.getByTestId('password-prompt')).toBeVisible({ timeout: 20_000 });
      await h1.page.getByTestId('password-prompt-input').click();
      await humanType(h1.page, 'wrong-pass');
      await h1.page.getByTestId('password-prompt-submit').click();
      await expect(h1.page.getByTestId('password-prompt-error')).toBeVisible({ timeout: 20_000 });
      await expect(h1.page.getByTestId('password-prompt-error')).not.toHaveText('');
      // 弹窗仍在，且拿不到频道内的私密消息
      await expect(h1.page.getByTestId('password-prompt')).toBeVisible();
      expect(await countMessageRows(h1.page, token)).toBe(0);

      // 改输正确密码
      const input = h1.page.getByTestId('password-prompt-input');
      await input.click();
      await input.press('Control+A');
      await humanType(h1.page, secret, { delay: 20 });
      await h1.page.getByTestId('password-prompt-submit').click();
      await expect(h1.page.getByTestId('password-prompt')).toHaveCount(0, { timeout: 20_000 });
      await expect(h1.page.getByRole('heading', { name: `#${name}` })).toBeVisible({ timeout: 30_000 });
      await waitChannelGroup(h1.page, privId);

      // 创建者进入（若弹密码框就输入）
      await nodeByName(h0.page, name).click();
      const prompt = h0.page.getByTestId('password-prompt');
      if (await prompt.isVisible().catch(() => false)) {
        await h0.page.getByTestId('password-prompt-input').click();
        await humanType(h0.page, secret, { delay: 20 });
        await h0.page.getByTestId('password-prompt-submit').click();
      }
      await expect(h0.page.getByRole('heading', { name: `#${name}` })).toBeVisible({ timeout: 30_000 });
      await waitChannelGroup(h0.page, privId);

      // 两人都在频道里后，私密消息应实时送达
      await sendMessageHuman(h0.page, token);
      await expect(messageRowByText(h1.page, token)).toBeVisible({ timeout: 30_000 });

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
