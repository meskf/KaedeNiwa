import { test, expect } from '@playwright/test';
import {
  PW,
  spawnHumans,
  closeHumans,
  humanType,
  humanPause,
  openView,
  logoutViaUI,
  loginExisting,
  waitForWorkspace,
  hardErrors,
} from '../lib/core';

test.describe('真人入门：注册 / 登录 / 会话持久化 / 错误路径', () => {
  test('注册 → 看到工作区 → 退出 → 密码输错 → 正确登录 → 刷新仍在线', async ({ browser }) => {
    const [h] = await spawnHumans(browser, 1, 'onb');
    try {
      await expect(h.page.getByTestId('message-input')).toBeVisible();
      await humanPause(h.page, 600, 1200);

      // 真人在设置页确认自己的账号，然后退出
      await openView(h.page, 'settings');
      await expect(h.page.getByTestId('settings-login-id')).toHaveText(h.name);
      await logoutViaUI(h.page);
      await expect(h.page.getByTestId('login-id')).toBeVisible();

      // 手滑输错密码：错误必须显示在登录页，不能进入工作区
      await h.page.getByTestId('login-id').click();
      await humanType(h.page, h.name);
      await h.page.getByTestId('login-password').click();
      await humanType(h.page, 'wrongpass123');
      await h.page.getByTestId('login-submit').click();
      await expect(h.page.getByText('Invalid account or password.', { exact: false })).toBeVisible({
        timeout: 20_000,
      });
      await expect(h.page.getByTestId('message-input')).toHaveCount(0);

      // 重新用正确密码登录
      await loginExisting(h.page, h.name, PW);

      // 刷新页面：会话应自动恢复，不需要重新登录
      await h.page.reload();
      await waitForWorkspace(h.page);
      await expect(h.page.getByTestId('message-input')).toBeVisible();
      await expect(h.page.getByTestId('login-id')).toHaveCount(0);

      expect(hardErrors(h)).toEqual([]);
    } finally {
      await closeHumans([h]);
    }
  });

  test('空账号/空密码：前端必须拦截并提示，不发请求', async ({ browser }) => {
    const [h] = await spawnHumans(browser, 1, 'onb-empty');
    try {
      await logoutViaUI(h.page);
      const requests: string[] = [];
      h.page.on('request', r => {
        if (/\/api\/(login|register)$/.test(r.url())) requests.push(r.url());
      });

      await h.page.getByTestId('login-submit').click();
      await expect(
        h.page.getByText(/Please enter account and password|请输入账号和密码/).first(),
      ).toBeVisible({ timeout: 10_000 });
      expect(requests).toEqual([]);
      expect(hardErrors(h)).toEqual([]);
    } finally {
      await closeHumans([h]);
    }
  });

  test('重复注册已存在的账号：明确提示“已被占用”，不会误登入', async ({ browser }) => {
    const [h] = await spawnHumans(browser, 1, 'onb-dup');
    try {
      await logoutViaUI(h.page);
      const toggle = h.page.getByTestId('login-toggle-register');
      await toggle.click();
      await h.page.getByTestId('login-id').click();
      await humanType(h.page, h.name);
      await h.page.getByTestId('login-password').click();
      await humanType(h.page, PW);
      await h.page.getByTestId('login-submit').click();

      await expect(h.page.getByText(/already taken/i).first()).toBeVisible({ timeout: 20_000 });
      await expect(h.page.getByTestId('message-input')).toHaveCount(0);

      // 真人改回登录模式，用正确密码进入
      const back = h.page.getByTestId('login-toggle-register');
      if (await back.isVisible().catch(() => false)) await back.click();
      await loginExisting(h.page, h.name, PW);
      await expect(h.page.getByTestId('message-input')).toBeVisible();
      expect(hardErrors(h)).toEqual([]);
    } finally {
      await closeHumans([h]);
    }
  });
});
