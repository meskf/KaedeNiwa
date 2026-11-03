import { test, expect } from '@playwright/test';
import { spawnUsers, closeUsers, syncRoom, switchView, ensureOp5 } from './lib/fixtures';
import { setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

// 1x1 JPEG（服务端要求 JPEG magic + 体积上限）
const TINY_JPEG = Buffer.from(
  '/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q==',
  'base64',
);

test.describe.configure({ mode: 'serial' });

test.describe('HUMAN SETTINGS & ADMIN', () => {
  test('01 profile, avatar upload, sessions panel', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 1, 'pf');
    const col = attachErrorCollector(users[0].page, 'u0');
    try {
      const u = users[0];
      await switchView(u.page, 'settings');

      // 会话面板：当前会话存在，撤销其他会话按钮不可用（只有一台）
      await expect(u.page.getByTestId('settings-sessions')).toBeVisible({ timeout: 15_000 });
      await expect(u.page.getByTestId('settings-session-current').first()).toBeVisible({ timeout: 15_000 });
      await expect(u.page.getByTestId('settings-revoke-others')).toBeDisabled();

      // 头像上传：等待一次 200 的 avatar 响应
      const resp = u.page.waitForResponse(
        r => r.url().includes(`/api/avatars/${u.name}`) && r.status() === 200,
        { timeout: 20_000 },
      );
      await u.page.getByTestId('settings-avatar-file').setInputFiles({
        name: 'avatar.jpg',
        mimeType: 'image/jpeg',
        buffer: TINY_JPEG,
      });
      await resp;

      // 昵称/简介/称号保存
      const nick = `nick-${tag()}`;
      await u.page.getByTestId('settings-nickname-input').click();
      await u.page.getByTestId('settings-nickname-input').press('Control+A');
      await u.page.keyboard.type(nick, { delay: 20 });
      await u.page.getByTestId('settings-profile-save').click();
      await expect(u.page.getByText(/已保存|Saved/).first()).toBeVisible({ timeout: 15_000 });

      const errs = col.getFiltered();
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('02 owner edits server profile and member op; member cannot escalate', async ({ browser }) => {
    test.setTimeout(240_000);
    const users = await spawnUsers(browser, 2, 'ad');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [owner, member] = users;
      await setupIsolatedChannel(users, 'iso');
      // e2e 库可能已有别的 op5：先用管理面把 owner 提升为 op5。
      await ensureOp5(owner.name);

      // owner (op5) 改服务器名
      await switchView(owner.page, 'settings');
      const newName = `srv-${tag()}`;
      await owner.page.getByTestId('settings-server-name').click();
      await owner.page.getByTestId('settings-server-name').press('Control+A');
      await owner.page.keyboard.type(newName, { delay: 20 });
      await owner.page.getByTestId('settings-server-save').click();
      await expect(owner.page.getByText(/已保存|Saved/).first()).toBeVisible({ timeout: 15_000 });

      // owner 可把 member 从 op3 降到 op2（等下用它验证权限拦截）
      const opSelect = owner.page.getByTestId(`member-op-${member.name}`);
      await expect(opSelect).toBeEnabled({ timeout: 15_000 });
      await opSelect.selectOption('op2');
      await expect(opSelect).toHaveValue('op2', { timeout: 10_000 });

      // member (op2) 打开设置：不能改 owner 的 op（op5 保护）
      await switchView(member.page, 'settings');
      await expect(member.page.getByTestId(`member-op-${owner.name}`)).toBeDisabled({ timeout: 15_000 });
      // op2 也不能保存服务器信息
      await member.page.getByTestId('settings-server-desc').click();
      await member.page.keyboard.type('nope', { delay: 15 });
      await member.page.getByTestId('settings-server-save').click();
      await expect(member.page.getByText(/需要管理员权限|Requires admin/).first()).toBeVisible({ timeout: 10_000 });

      // 复位 member 为 op3，避免影响后续用例
      await opSelect.selectOption('op3');
      await expect(opSelect).toHaveValue('op3', { timeout: 10_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});

