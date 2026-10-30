import { test as base, expect, Page, BrowserContext } from '@playwright/test';

export const randName = (p: string) => `${p}-${Math.floor(Math.random() * 1_000_000)}`;

export async function login(page: Page, loginId: string, password = 'password123') {
  await page.goto('/');
  await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 20_000 });
  await page.getByTestId('login-id').fill(loginId);
  await page.getByTestId('login-password').fill(password);
  const reg = page.getByTestId('login-toggle-register');
  if (await reg.isVisible()) await reg.click();
  await page.getByTestId('login-submit').click();
  await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 20_000 });
}

export async function sendMessage(page: Page, text: string) {
  await page.getByTestId('message-input').click();
  await page.keyboard.type(text);
  await page.keyboard.press('Enter');
}

export async function waitDebug(page: Page, timeoutMs = 15_000) {
  await page.waitForFunction(() => !!(window as any).__kaede_debug__, null, { timeout: timeoutMs });
}

export async function getFirstTextChannelId(page: Page): Promise<string | null> {
  return await page.evaluate(async () => {
    const dbg = (window as any).__kaede_debug__;
    if (!dbg || !dbg.channels) return null;
    try {
      const chList = await dbg.channels.list();
      const textCh = chList.find((c: any) => c.kind === 'text');
      return textCh ? textCh.id : null;
    } catch { return null; }
  });
}

type Fixtures = { u0: Page; u1: Page; u0Name: string; u1Name: string; u0Ctx: BrowserContext; u1Ctx: BrowserContext; };

export const test = base.extend<Fixtures>({
  u0Name: async ({}, use) => use(randName('u0')),
  u1Name: async ({}, use) => use(randName('u1')),
  u0Ctx: async ({ browser }, use) => { const ctx = await browser.newContext({ ignoreHTTPSErrors: true }); await use(ctx); await ctx.close(); },
  u1Ctx: async ({ browser }, use) => { const ctx = await browser.newContext({ ignoreHTTPSErrors: true }); await use(ctx); await ctx.close(); },
  u0: async ({ u0Ctx, u0Name }, use) => { const page = await u0Ctx.newPage(); await login(page, u0Name); await use(page); },
  u1: async ({ u1Ctx, u1Name }, use) => { const page = await u1Ctx.newPage(); await login(page, u1Name); await use(page); },
});

export { expect };
