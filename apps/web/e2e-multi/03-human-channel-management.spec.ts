import { test, expect } from '@playwright/test';
import {
  spawnUsers,
  closeUsers,
  sendMessage,
  syncRoom,
  switchView,
  createNodeViaUI,
  waitForNode,
  nodeByName,
  renameNodeViaUI,
  deleteNodeViaUI,
  clickChannel,
  loginExisting,
} from './lib/fixtures';
import { setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

test.describe.configure({ mode: 'serial' });

test.describe('HUMAN CHANNEL MANAGEMENT', () => {
  test('01 create folder + text channel via context menu, visible to peers', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'ch');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [owner, peer] = users;
      await setupIsolatedChannel(users, 'iso');

      const t = tag();
      const folder = `folder-${t}`;
      const channel = `room-${t}`;

      await createNodeViaUI(owner.page, { name: folder, kind: 'folder' });
      await waitForNode(owner.page, folder);
      await createNodeViaUI(owner.page, { name: channel, kind: 'text', parent: folder });
      await waitForNode(owner.page, channel);

      // 另一端没有节点广播，靠 10s 轮询同步。20s 内必须出现。
      await waitForNode(peer.page, folder, { timeout: 25_000 });
      await waitForNode(peer.page, channel, { timeout: 25_000 });

      // peer 进入新频道发消息，owner 仍在 general，不应收到（频道隔离）
      await clickChannel(peer.page, channel);
      await expect(peer.page.getByRole('heading', { name: `#${channel}` })).toBeVisible({ timeout: 15_000 });
      const secret = `only-in-${channel}`;
      await sendMessage(peer.page, secret);
      await peer.page.waitForTimeout(2_500);

      expect(await owner.page.getByText(secret, { exact: false }).count(), '频道隔离失败：owner 看到了新频道消息').toBe(0);

      // owner 进入同一频道。E2EE 语义：加入前历史不可解密（频道 key 已随 epoch 轮换），
      // 因此只验证加入之后的实时消息互通。
      await clickChannel(owner.page, channel);
      await expect(owner.page.getByRole('heading', { name: `#${channel}` })).toBeVisible({ timeout: 15_000 });
      await owner.page.waitForTimeout(2_500);

      expect(
        await owner.page.getByText(secret, { exact: false }).count(),
        '加入前的历史不应被新成员解密（前向保密）',
      ).toBe(0);

      const live = `live-${t}`;
      await sendMessage(peer.page, live);
      await expect(owner.page.getByText(live, { exact: false }).first()).toBeVisible({ timeout: 25_000 });
      await owner.page.getByTestId('message-input').click();
      await owner.page.keyboard.type(`reply-${t}`, { delay: 20 });
      await owner.page.keyboard.press('Enter');
      await expect(peer.page.getByText(`reply-${t}`, { exact: false }).first()).toBeVisible({ timeout: 20_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('02 rename and delete channel via context menu propagates to peers', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'rn');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [owner, peer] = users;
      await setupIsolatedChannel(users, 'iso');

      const t = tag();
      const name = `ren-${t}`;
      const renamed = `renamed-${t}`;

      await createNodeViaUI(owner.page, { name, kind: 'text' });
      await waitForNode(owner.page, name);

      await renameNodeViaUI(owner.page, name, renamed);
      await waitForNode(owner.page, renamed);
      await waitForNode(peer.page, renamed, { timeout: 25_000 });

      await deleteNodeViaUI(owner.page, renamed);
      await nodeByName(owner.page, renamed).waitFor({ state: 'detached', timeout: 15_000 });
      await nodeByName(peer.page, renamed).waitFor({ state: 'detached', timeout: 25_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('03 settings page updates nickname, theme, language and logs out', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 1, 'st');
    const col = attachErrorCollector(users[0].page, 'u0');
    try {
      const u = users[0];
      await switchView(u.page, 'settings');

      // 昵称 / 简介 / 称号
      await u.page.getByTestId('settings-nickname-input').click();
      await u.page.keyboard.type(`nick-${tag()}`, { delay: 20 });
      await u.page.getByTestId('settings-bio-input').click();
      await u.page.keyboard.type('bio-from-human-test', { delay: 15 });
      await u.page.getByTestId('settings-title-input').click();
      await u.page.keyboard.type('qa', { delay: 15 });
      await u.page.getByTestId('settings-profile-save').click();
      await expect(u.page.getByText(/已保存|Saved/).first()).toBeVisible({ timeout: 10_000 });

      // 主题切换：light → dark，localStorage 持久化
      await u.page.getByTestId('settings-theme-dark').click();
      expect(await u.page.evaluate(() => localStorage.getItem('kaede-theme'))).toBe('dark');
      await u.page.getByTestId('settings-theme-light').click();
      expect(await u.page.evaluate(() => localStorage.getItem('kaede-theme'))).toBe('light');

      // 语言切换：中文 → 导航文案变化 → 切回英文
      await u.page.getByTestId('settings-lang-button').click();
      await u.page.getByTestId('settings-lang-zh').click();
      await expect(u.page.getByTestId('nav-dms')).toContainText('私信');
      await u.page.getByTestId('settings-lang-button').click();
      await u.page.getByTestId('settings-lang-en').click();
      await expect(u.page.getByTestId('nav-dms')).toContainText('DMs');

      // 登出 → 回到登录页 → 已存在账号重新登录
      await u.page.getByTestId('settings-logout').click();
      await expect(u.page.getByTestId('login-id')).toBeVisible({ timeout: 15_000 });
      await loginExisting(u.page, u.name);
      await expect(u.page.getByTestId('message-input')).toBeVisible({ timeout: 20_000 });

      const errs = col.getFiltered();
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});

