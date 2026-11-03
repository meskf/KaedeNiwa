import { setupIsolatedChannel } from './lib/fixtures';
import { test, expect } from '@playwright/test';
import {
  spawnUsers,
  closeUsers,
  sendMessage,
  syncRoom,
  switchView,
  countMessageRows,
  loginExisting,
  waitForNode,
  clickChannel,
} from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

test.describe.configure({ mode: 'serial' });

test.describe('HUMAN 5-USER RELIABILITY', () => {
  test('5 users, page reload + relogin mid-run, every message exactly once everywhere', async ({ browser }) => {
    test.setTimeout(420_000);
    const users = await spawnUsers(browser, 5, 'm5');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const ch = await setupIsolatedChannel(users, 'm5');

      const t = tag();
      const sent: string[] = [];
      const rounds = 3;

      for (let r = 0; r < rounds; r++) {
        for (let i = 0; i < users.length; i++) {
          // 第二轮中途让 u2 刷新页面（真实用户误触刷新 / 浏览器崩溃恢复）
          if (r === 1 && i === 0) {
            await users[2].page.reload();
            await users[2].page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 40_000 });
            await users[2].page.waitForFunction(() => !!(window as any).__kaede_debug__, null, { timeout: 25_000 });
            // 刷新后 currentChannel 回到默认 general，需要重新点回测试频道
            await waitForNode(users[2].page, ch.name, { timeout: 40_000 });
            await clickChannel(users[2].page, ch.name);
            await expect(users[2].page.getByRole('heading', { name: `#${ch.name}` })).toBeVisible({ timeout: 20_000 });
            await users[2].page.waitForTimeout(3_000);
          }
          // 第三轮中途让 u4 登出再登录
          if (r === 2 && i === 1) {
            await switchView(users[4].page, 'settings');
            await users[4].page.getByTestId('settings-logout').click();
            await users[4].page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 20_000 });
            await loginExisting(users[4].page, users[4].name);
            await waitForNode(users[4].page, ch.name, { timeout: 40_000 });
            await clickChannel(users[4].page, ch.name);
            await expect(users[4].page.getByRole('heading', { name: `#${ch.name}` })).toBeVisible({ timeout: 20_000 });
            await users[4].page.waitForTimeout(3_000);
          }

          const text = `m5-${t}-r${r}-u${i}`;
          sent.push(text);
          await sendMessage(users[i].page, text);
          await users[i].page.waitForTimeout(120);
        }
      }

      // 等待网络收敛
      await users[0].page.waitForTimeout(6_000);

      for (let i = 0; i < users.length; i++) {
        for (const text of sent) {
          await expect(
            users[i].page.getByText(text, { exact: false }).first(),
            `u${i} missing ${text}`,
          ).toBeVisible({ timeout: 30_000 });
          const c = await countMessageRows(users[i].page, text);
          expect(c, `u${i} duplicate ${text} (count=${c})`).toBe(1);
        }
      }

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});

