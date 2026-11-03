import { test, expect } from '@playwright/test';
import { spawnUsers, closeUsers, sendMessage, syncRoom, switchView } from './lib/fixtures';
import { setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

test.describe.configure({ mode: 'serial' });

test.describe('HUMAN FRIENDS & DM', () => {
  test('add friend, accept, DM both ways, remove friend', async ({ browser }) => {
    test.setTimeout(240_000);
    const users = await spawnUsers(browser, 2, 'fr');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'iso');

      // A 在好友页发起请求
      await switchView(a.page, 'friends');
      await a.page.getByTestId('friends-add-open').click();
      await a.page.getByTestId('friends-add-input').click();
      await a.page.keyboard.type(b.name, { delay: 20 });
      await a.page.getByTestId('friends-add-submit').click();
      await expect(a.page.getByTestId(`friend-row-${b.name}`)).toBeVisible({ timeout: 15_000 });

      // B 在好友页接受（FriendsChanged 事件会自动刷新列表）
      await switchView(b.page, 'friends');
      const accept = b.page.getByTestId(`friend-accept-${a.name}`);
      await expect(accept).toBeVisible({ timeout: 20_000 });
      await accept.click();

      // A 侧状态变为好友，并出现 Message 操作
      const msgBtn = a.page.getByTestId(`friend-message-${b.name}`);
      await expect(msgBtn).toBeVisible({ timeout: 20_000 });

      // A 点 Message → 进入 DM 会话并发消息
      await msgBtn.click();
      await expect(a.page.getByTestId(`dm-conv-${b.name}`)).toBeVisible({ timeout: 15_000 });
      const t = tag();
      const dm1 = `dm-a-${t}`;
      await sendMessage(a.page, dm1);
      await expect(a.page.getByTestId('dm-message').filter({ hasText: dm1 }).first()).toBeVisible({ timeout: 15_000 });

      // B 进入 DM 页，打开同一会话，看到消息
      await switchView(b.page, 'dms');
      await b.page.getByTestId(`dm-conv-${a.name}`).click();
      await expect(b.page.getByTestId('dm-message').filter({ hasText: dm1 }).first()).toBeVisible({ timeout: 20_000 });

      // B 回复，A 收到
      const dm2 = `dm-b-${t}`;
      await sendMessage(b.page, dm2);
      await expect(a.page.getByTestId('dm-message').filter({ hasText: dm2 }).first()).toBeVisible({ timeout: 20_000 });

      // A 删除好友，双方列表更新
      await switchView(a.page, 'friends');
      await a.page.getByTestId(`friend-row-${b.name}`).hover();
      await a.page.getByTestId(`friend-remove-${b.name}`).click();
      await expect(a.page.getByTestId(`friend-row-${b.name}`)).toHaveCount(0, { timeout: 20_000 });
      await expect(b.page.getByTestId(`friend-row-${a.name}`)).toHaveCount(0, { timeout: 20_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('friend request reject', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'rj');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'iso');

      await switchView(a.page, 'friends');
      await a.page.getByTestId('friends-add-open').click();
      await a.page.getByTestId('friends-add-input').click();
      await a.page.keyboard.type(b.name, { delay: 20 });
      await a.page.getByTestId('friends-add-submit').click();

      await switchView(b.page, 'friends');
      const reject = b.page.getByTestId(`friend-reject-${a.name}`);
      await expect(reject).toBeVisible({ timeout: 20_000 });
      await reject.click();

      await expect(b.page.getByTestId(`friend-row-${a.name}`)).toHaveCount(0, { timeout: 20_000 });
      await expect(a.page.getByTestId(`friend-row-${b.name}`)).toHaveCount(0, { timeout: 20_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});

