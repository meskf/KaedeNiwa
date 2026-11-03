import { test, expect } from '@playwright/test';
import {
  spawnUsers,
  closeUsers,
  sendMessage,
  syncRoom,
  messageRowByText,
  countMessageRows,
} from './lib/fixtures';
import { setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

test.describe.configure({ mode: 'serial' });

test.describe('HUMAN MESSAGING FEATURES', () => {
  test('01 reaction via right-click context menu, peer sees pill, toggle removes', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'rx');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'iso');

      const t = tag();
      const text = `react-target-${t}`;
      await sendMessage(a.page, text);
      await expect(messageRowByText(b.page, text)).toBeVisible({ timeout: 20_000 });

      // A 右键自己的消息 → 快捷回应 👍
      await messageRowByText(a.page, text).click({ button: 'right' });
      await expect(a.page.getByTestId('message-context-menu')).toBeVisible();
      await a.page.getByTestId('quick-react-👍').click();

      const pillB = b.page.locator('[data-testid="reaction-pill-👍"]').first();
      await expect(pillB).toBeVisible({ timeout: 15_000 });
      await expect(pillB).toHaveAttribute('data-count', '1');

      // B 再补一个 ❤️：A 应看到两个 pill
      await messageRowByText(b.page, text).click({ button: 'right' });
      await b.page.getByTestId('quick-react-❤️').click();
      const pillA = a.page.locator('[data-testid="reaction-pill-👍"]').first();
      await expect(pillA).toHaveAttribute('data-mine', '1', { timeout: 15_000 });
      await expect(a.page.locator('[data-testid="reaction-pill-❤️"]').first()).toBeVisible({ timeout: 15_000 });

      // A 点自己的 👍 pill 取消，B 侧的 👍 应消失（唯一用户被移除）
      await pillA.click();
      await expect(b.page.locator('[data-testid="reaction-pill-👍"]')).toHaveCount(0, { timeout: 15_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('02 typing indicator appears and expires', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'ty');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'iso');

      const input = a.page.getByTestId('message-input');
      await input.click();
      await a.page.keyboard.type('typing-but-not-sent', { delay: 40 });

      const indicator = b.page.getByTestId('typing-indicator');
      await expect(indicator).toBeVisible({ timeout: 10_000 });
      await expect(indicator).toContainText(a.name);

      // onTyping 每次输入最多 2s 发一次，服务端 ttl≈3.5s；不再输入后会消失
      await expect(indicator).toBeHidden({ timeout: 15_000 });

      // 清理输入，避免后续测试串内容
      await input.click();
      await input.press('Control+A');
      await input.press('Delete');

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('03 multiline message and rapid consecutive sends are not duplicated', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'ml');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'iso');

      const t = tag();
      const line1 = `line1-${t}`;
      const line2 = `line2-${t}`;
      await a.page.getByTestId('message-input').click();
      await a.page.keyboard.type(line1, { delay: 20 });
      await a.page.keyboard.press('Shift+Enter');
      await a.page.keyboard.type(line2, { delay: 20 });
      await a.page.keyboard.press('Enter');

      await expect(b.page.getByText(line1, { exact: false }).first()).toBeVisible({ timeout: 20_000 });
      await expect(b.page.getByText(line2, { exact: false }).first()).toBeVisible({ timeout: 20_000 });
      // 两行必须属于同一条消息
      expect(await countMessageRows(b.page, line1)).toBe(1);
      expect(await countMessageRows(b.page, line2)).toBe(1);

      // 快速连发 8 条：全部到达且每条恰好一次
      const rapid: string[] = [];
      for (let i = 0; i < 8; i++) {
        const text = `rapid-${t}-${i}`;
        rapid.push(text);
        await a.page.getByTestId('message-input').click();
        await a.page.keyboard.type(text, { delay: 5 });
        await a.page.keyboard.press('Enter');
      }
      for (const text of rapid) {
        await expect(b.page.getByText(text, { exact: false }).first()).toBeVisible({ timeout: 25_000 });
        // 服务端先 fanout 给对端、后返回 POST 响应，发送方自己的落库会略晚。
        await expect(messageRowByText(a.page, text)).toBeVisible({ timeout: 25_000 });
        expect(await countMessageRows(b.page, text), `duplicate of ${text}`).toBe(1);
        expect(await countMessageRows(a.page, text), `sender-side duplicate of ${text}`).toBe(1);
      }

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});

