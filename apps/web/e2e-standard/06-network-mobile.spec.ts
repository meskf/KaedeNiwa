import { test, expect, waitDebug, getFirstTextChannelId, sendMessage } from './fixtures';
import { devices } from '@playwright/test';

test.describe('NETWORK', () => {
  test('断线重连：模拟离线后再恢复，消息能正常同步', async ({ browser, u0Name }) => {
    // 只用一个用户，方便控制网络
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
    const page = await ctx.newPage();
    
    // 登录
    await page.goto('/');
    await page.getByTestId('login-id').fill(u0Name);
    await page.getByTestId('login-password').fill('password123');
    await page.getByTestId('login-toggle-register').click();
    await page.getByTestId('login-submit').click();
    await page.getByTestId('message-input').waitFor({ state: 'visible' });
    
    await waitDebug(page);
    await page.waitForTimeout(2000);

    const chId = await getFirstTextChannelId(page);
    expect(chId).toBeTruthy();

    // 1. 正常发一条
    await sendMessage(page, 'BEFORE_DISCONNECT');
    await page.waitForTimeout(1000);

    // 2. 切断网络
    await ctx.setOffline(true);
    await page.waitForTimeout(2000); // 触发离线检测

    // 3. 恢复网络
    await ctx.setOffline(false);
    await page.waitForTimeout(3000); // 等待重连机制生效

    // 4. 再次发送，验证底层连接已恢复
    const msgAfter = `AFTER_RECONNECT_${Date.now()}`;
    await sendMessage(page, msgAfter);
    await page.waitForTimeout(2000);

    // 5. 验证消息能存进历史 (说明确实重连并发送成功了)
    const history = await page.evaluate((ch) => (window as any).__kaede_debug__.loadHistory(ch, 20), chId);
    expect(JSON.stringify(history)).toContain(msgAfter);

    await ctx.close();
  });
});

test.describe('MOBILE UI', () => {
  test('移动端首屏渲染与发消息', async ({ browser, u0Name }) => {
    // 这里直接传入 devices['Pixel 5'] 的配置
    const ctx = await browser.newContext({ ...devices['Pixel 5'], ignoreHTTPSErrors: true });
    const page = await ctx.newPage();
    
    await page.goto('/');
    await page.getByTestId('login-id').fill(u0Name);
    await page.getByTestId('login-password').fill('password123');
    await page.getByTestId('login-toggle-register').click();
    await page.getByTestId('login-submit').click();
    await page.getByTestId('message-input').waitFor({ state: 'visible' });

    // 验证底部 Tab 导航存在
    await expect(page.getByTestId('nav-chat')).toBeVisible();
    await expect(page.getByTestId('nav-dms')).toBeVisible();
    await expect(page.getByTestId('nav-friends')).toBeVisible();

    // 移动端发消息测试
    const mobileMsg = `MOBILE_MSG_${Date.now()}`;
    await sendMessage(page, mobileMsg);
    await expect(page.getByText(mobileMsg, { exact: false }).first()).toBeVisible({ timeout: 10000 });

    await ctx.close();
  });
});
