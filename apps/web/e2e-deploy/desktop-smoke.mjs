// 桌面客户端冒烟：连接已启动的 Electron（--remote-debugging-port=9222），
// 走真实 UI 完成 注册/登录/频道列表/发消息。用法：
//   node e2e-deploy/desktop-smoke.mjs   （cwd = apps/web）
import { chromium } from '@playwright/test';

const CDP = process.env.KAEDE_CDP ?? 'http://127.0.0.1:9222';
const loginId = `desktop-${Date.now().toString(36).slice(-6)}`;

const browser = await chromium.connectOverCDP(CDP);
const ctx = browser.contexts()[0];
const page = ctx.pages()[0] ?? await ctx.waitForEvent('page');
console.log('[desktop] page url:', page.url());

await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 60_000 });
await page.getByTestId('login-server-url').waitFor({ state: 'visible', timeout: 10_000 }).catch(() => {});

// 注册（登录页默认登录模式，点“注册”切换）
const toggle = page.getByTestId('login-toggle-register');
if (await toggle.isVisible()) await toggle.click();
await page.getByTestId('login-id').fill(loginId);
await page.getByTestId('login-password').fill('password123');
await page.getByTestId('login-submit').click();

await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 60_000 });
await page.waitForFunction(() => !!(window).__kaede_debug__, null, { timeout: 30_000 });
const chans = await page.evaluate(async () => __kaede_debug__.channels.list());
if (!Array.isArray(chans) || chans.length === 0) throw new Error('no channels: ' + JSON.stringify(chans));
console.log('[desktop] channels:', chans.map(c => `${c.kind}:${c.name}`).join(', '));

// 进入第一个文字频道并发消息
const firstText = chans.find(c => c.kind !== 'voice');
if (firstText) {
  const byId = page.locator(`[data-node-id="${firstText.id}"]`);
  if (await byId.count()) await byId.first().click();
  else await page.locator('aside').first().getByText(firstText.name, { exact: true }).first().click();
}
await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 30_000 });
const text = `desktop-smoke-${Date.now().toString(36)}`;
await page.getByTestId('message-input').click();
await page.getByTestId('message-input').pressSequentially(text, { delay: 30 });
await page.getByTestId('message-input').press('Enter');
await page.locator('[data-testid="message-row"]').filter({ hasText: text }).first()
  .waitFor({ state: 'visible', timeout: 30_000 });
console.log(`[desktop] OK user=${loginId} message=${text}`);

await browser.close();
