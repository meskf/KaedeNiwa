import { Browser, BrowserContext, Page, expect } from '@playwright/test';
import { attachErrorCollector, ErrorCollector } from './observe';

export const PW = 'password123';
export const randName = (p: string) => `${p}-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6)}`;

export type Human = {
  ctx: BrowserContext;
  page: Page;
  name: string;
  errors: ErrorCollector;
};

export type View = 'chat' | 'dms' | 'friends' | 'settings' | 'more';

export type SpawnOptions = {
  /** 登录导航前注入脚本（如 getDisplayMedia 失败注入） */
  beforeLogin?: (page: Page) => Promise<void>;
  permissions?: string[];
  viewport?: { width: number; height: number };
};

// ---------------------------------------------------------------------------
// 真人输入：逐字符键盘事件 + 随机停顿；中文/emoji 用 insertText 走 input 事件
// ---------------------------------------------------------------------------

async function typeText(page: Page, text: string, delay = 28, pauses = true) {
  for (const ch of Array.from(text)) {
    if (ch === '\n') {
      await page.keyboard.press('Shift+Enter');
      continue;
    }
    if (ch.codePointAt(0)! > 0xffff || /[^\x00-\x7F]/.test(ch)) {
      await page.keyboard.insertText(ch);
    } else {
      await page.keyboard.type(ch);
    }
    await page.waitForTimeout(delay + Math.round(Math.random() * 22));
    if (pauses && /[，。！？、,.!?]/.test(ch)) {
      await page.waitForTimeout(90 + Math.round(Math.random() * 200));
    }
  }
}

/** 逐字输入（模拟真人打字节奏）。需要先让目标输入框获得焦点。 */
export async function humanType(page: Page, text: string, opts: { delay?: number; pauses?: boolean } = {}) {
  await typeText(page, text, opts.delay ?? 28, opts.pauses !== false);
}

/** 先打错一个字符、再退格改正——模拟真实用户的手误。 */
export async function humanTypeWithFix(page: Page, text: string) {
  const cut = Math.max(1, Math.floor(text.length * 0.4));
  await humanType(page, text.slice(0, cut));
  await page.keyboard.type('x');
  await page.waitForTimeout(280);
  await page.keyboard.press('Backspace');
  await page.waitForTimeout(120);
  await humanType(page, text.slice(cut));
}

/** 硬错误：未捕获异常 + 服务端 5xx。这类必须为零。 */
export function hardErrors(h: Human): string[] {
  return h.errors.get().filter(e => e.includes('pageerror') || /\bhttp 5\d\d\b/.test(e));
}

/** 像真人一样阅读/确认界面再动作。 */
export async function humanPause(page: Page, min = 250, max = 900) {
  await page.waitForTimeout(min + Math.round(Math.random() * (max - min)));
}

export async function humanClick(page: Page, locator: ReturnType<Page['locator']>) {
  await locator.hover().catch(() => undefined);
  await humanPause(page, 120, 320);
  await locator.click();
}

// ---------------------------------------------------------------------------
// 登录 / 注册 / 登出
// ---------------------------------------------------------------------------

export async function waitForWorkspace(page: Page) {
  await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 30_000 });
  await page.waitForFunction(() => !!(window as any).__kaede_debug__, null, { timeout: 25_000 });
  // 同样不能用 async waitForFunction：Promise 会被当成 truthy 直接通过。
  await expect
    .poll(
      async () => {
        const count = await page.evaluate(async () => {
          const dbg = (window as any).__kaede_debug__;
          if (!dbg?.channels?.list) return 0;
          try {
            const list = await dbg.channels.list();
            return Array.isArray(list) ? list.length : 0;
          } catch {
            return 0;
          }
        });
        return count;
      },
      { timeout: 30_000, intervals: [500, 1000] },
    )
    .toBeGreaterThan(0);
}

/**
 * 真人注册。输入全部走键盘；若服务端限流/报错，抛错由 spawnHumans 重试。
 * register=false 时为“已注册账号登录”，不点注册 toggle。
 */
export async function loginHuman(
  page: Page,
  loginId: string,
  password = PW,
  opts: { register?: boolean } = {},
) {
  const register = opts.register !== false;
  await page.goto('/');
  const idBox = page.getByTestId('login-id');
  await idBox.waitFor({ state: 'visible', timeout: 30_000 });
  await idBox.click();
  await humanType(page, loginId, { delay: 18 });
  const pwBox = page.getByTestId('login-password');
  await pwBox.click();
  await humanType(page, password, { delay: 16 });
  if (register) {
    const reg = page.getByTestId('login-toggle-register');
    if (await reg.isVisible().catch(() => false)) await reg.click();
  }
  await page.getByTestId('login-submit').click();

  const result = await Promise.race([
    page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 15_000 }).then(() => 'ok').catch(() => 'timeout'),
    page
      .getByText(/internal error|rate limit|login_id_taken|Invalid account|already taken/i)
      .first()
      .waitFor({ state: 'visible', timeout: 15_000 })
      .then(() => 'err')
      .catch(() => 'timeout'),
  ]);
  if (result === 'err') {
    const msg = await page
      .getByText(/internal error|rate limit|login_id_taken|Invalid account|already taken/i)
      .first()
      .textContent();
    throw new Error(`login/register failed: ${msg}`);
  }
  if (result === 'timeout') throw new Error(`login/register timeout (rate limit?): user=${loginId}`);
  await waitForWorkspace(page);
}

export async function loginExisting(page: Page, loginId: string, password = PW) {
  await loginHuman(page, loginId, password, { register: false });
}

/** 从设置页点“退出登录”。 */
export async function logoutViaUI(page: Page) {
  await openView(page, 'settings');
  await page.getByTestId('settings-logout').click();
  await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 20_000 });
}

// ---------------------------------------------------------------------------
// 用户池
// ---------------------------------------------------------------------------

/**
 * 生成 N 个独立浏览器上下文的真人用户并完成注册。
 * 每个 attempt 使用全新账号（避免“注册已成功但跳转超时”后重试撞 login_id_taken）。
 */
export async function spawnHumans(
  browser: Browser,
  count: number,
  prefix = 'h',
  opts: SpawnOptions = {},
): Promise<Human[]> {
  const humans: Human[] = [];
  for (let i = 0; i < count; i++) {
    let lastErr: Error | null = null;
    for (let attempt = 0; attempt < 3; attempt++) {
      const name = randName(`${prefix}${i}`);
      const ctx = await browser.newContext({
        ignoreHTTPSErrors: true,
        permissions: opts.permissions ?? ['camera', 'microphone'],
        viewport: opts.viewport ?? { width: 1440, height: 900 },
        acceptDownloads: true,
      });
      const page = await ctx.newPage();
      const errors = attachErrorCollector(page, `${prefix}${i}`);
      try {
        if (opts.beforeLogin) await opts.beforeLogin(page);
        await loginHuman(page, name);
        humans.push({ ctx, page, name, errors });
        lastErr = null;
        break;
      } catch (e) {
        lastErr = e as Error;
        await ctx.close().catch(() => {});
        if (attempt < 2) {
          console.log(
            `[spawnHumans] ${name} attempt ${attempt + 1}/3 failed (${(e as Error).message?.slice(0, 120)}), retry in 2s`,
          );
          await new Promise(r => setTimeout(r, 2000));
        }
      }
    }
    if (lastErr) throw new Error(`spawnHumans: failed after 3 attempts: ${lastErr.message}`);
  }
  return humans;
}

/**
 * 关上下文前优先用 UI 的“离开语音”按钮走完整卸载，
 * 避免 abrupt close 留下幽灵 participant / 共享面板污染后续用例。
 */
export async function closeHumans(humans: Human[]) {
  for (const h of humans) {
    try {
      if (!h.page.isClosed()) {
        const connected = h.page.getByTestId('media-connected');
        if (await connected.isVisible().catch(() => false)) {
          const leave = h.page.getByTestId('media-leave');
          if (await leave.isVisible().catch(() => false)) {
            await leave.click();
            await h.page.waitForTimeout(600);
          }
        }
      }
    } catch {
      /* 页面已关闭/状态已变，忽略 */
    }
    await h.ctx.close().catch(() => undefined);
  }
}

// ---------------------------------------------------------------------------
// 视图 / 频道导航（真人点击）
// ---------------------------------------------------------------------------

export async function openView(page: Page, view: View) {
  await page.getByTestId(`nav-${view}`).click();
  await humanPause(page, 150, 400);
}

export function nodeByName(page: Page, name: string) {
  return page.locator('[data-node-id]').filter({ hasText: name }).first();
}

export async function waitForNode(
  page: Page,
  name: string,
  opts: { timeout?: number; state?: 'visible' | 'detached' } = {},
) {
  const node = nodeByName(page, name);
  await node.waitFor({ state: opts.state ?? 'visible', timeout: opts.timeout ?? 25_000 });
  return node;
}

export async function clickChannel(page: Page, name: string) {
  await nodeByName(page, name).click();
}

/** 从 store 里按名字查节点 id（避开 DOM 文本包含关系）。 */
export async function nodeId(page: Page, name: string): Promise<string> {
  const id = await page.evaluate(n => {
    const st = (window as any).__kaede_debug__.store.getState();
    return st.nodes.find((x: any) => x.name === n)?.id as string | undefined;
  }, name);
  if (!id) throw new Error(`nodeId(${name}) not found`);
  return id;
}

/** 等频道 key 分发完成（MLS group 就绪）再发消息，避免“消息发在还没组网时”。 */
export async function waitChannelGroup(page: Page, id: string) {
  // 注意：Playwright 的 waitForFunction 对 async 谓词不会 await 返回的 Promise
  //（Promise 本身是 truthy，会立刻通过）。这里必须用 expect.poll。
  await expect
    .poll(
      async () => {
        const st = await groupDiag(page, id);
        return !!(st && (st as any).has_group);
      },
      { timeout: 60_000, intervals: [500, 1000, 2000] },
    )
    .toBe(true);
}

/**
 * 等主群 MLS 就绪。
 * 公开频道的 reaction 走主群 MLS 广播（不是频道 group），
 * 主群没就绪时发送会失败、UI 会把乐观 reaction 回滚。
 */
export async function waitMainGroupReady(page: Page, timeoutMs = 45_000) {
  await expect
    .poll(
      async () => {
        const st = await groupDiag(page, 'general');
        return !!(st && (st as any).main_ready);
      },
      { timeout: timeoutMs, intervals: [500, 1000, 2000] },
    )
    .toBe(true);
}

/** 读取主群/频道 group 诊断（定位“能收不能发”这类半接入态）。 */
export async function groupDiag(page: Page, channelId = 'general') {
  return page.evaluate(async id => {
    const dbg = (window as any).__kaede_debug__;
    if (!dbg) return null;
    try {
      return await dbg.channelState(id);
    } catch (e) {
      return { error: String(e) };
    }
  }, channelId);
}

export async function clickChannelById(page: Page, id: string) {
  const node = page.locator(`[data-node-id="${id}"]`).first();
  await node.waitFor({ state: 'visible', timeout: 25_000 });
  await node.click();
}

/**
 * 通过真实 UI 菜单新建节点：
 *   - 根节点：点侧栏 “+” 按钮
 *   - 子节点：右键父节点
 */
export async function createNodeViaUI(
  page: Page,
  opts: { name: string; kind: 'folder' | 'text' | 'voice' | 'private'; password?: string; parent?: string },
) {
  if (opts.parent) {
    await nodeByName(page, opts.parent).click({ button: 'right' });
  } else {
    await page.getByTestId('sidebar-create-node').click();
  }
  await humanPause(page, 200, 500);
  const menuTestId =
    opts.kind === 'private'
      ? 'ctx-create-private'
      : opts.kind === 'text'
        ? 'ctx-create-text'
        : opts.kind === 'voice'
          ? 'ctx-create-voice'
          : 'ctx-create-folder';
  await page.getByTestId(menuTestId).click();

  const nameInput = page.locator('input[placeholder="输入名称..."]');
  await nameInput.click();
  await humanType(page, opts.name, { delay: 22 });
  if (opts.kind === 'private') {
    const pwInput = page.getByTestId('ctx-create-private-password');
    await pwInput.click();
    await humanType(page, opts.password || PW, { delay: 18 });
  }
  await page.getByTestId('ctx-confirm').click();
}

export async function renameNodeViaUI(page: Page, oldName: string, newName: string) {
  await nodeByName(page, oldName).click({ button: 'right' });
  await page.getByTestId('ctx-rename').click();
  const input = page.locator('input[placeholder="输入名称..."]');
  await input.click();
  await input.press('Control+A');
  await humanType(page, newName, { delay: 22 });
  await page.getByTestId('ctx-confirm').click();
}

export async function deleteNodeViaUI(page: Page, name: string) {
  await nodeByName(page, name).click({ button: 'right' });
  page.once('dialog', d => d.accept());
  await page.getByTestId('ctx-delete').click();
}

// ---------------------------------------------------------------------------
// 消息
// ---------------------------------------------------------------------------

export async function sendMessageHuman(page: Page, text: string) {
  const box = page.getByTestId('message-input');
  await box.click();
  await humanType(page, text);
  await humanPause(page, 180, 350);
  await page.keyboard.press('Enter');
}

/** 模拟“粘贴 + 回车”的连发节奏（不是逐字打）。 */
export async function pasteAndSend(page: Page, text: string) {
  const box = page.getByTestId('message-input');
  await box.click();
  await page.keyboard.insertText(text);
  await page.keyboard.press('Enter');
}

export function messageRowByText(page: Page, text: string) {
  return page.locator('[data-testid="message-row"]').filter({ hasText: text }).first();
}

export async function countMessageRows(page: Page, text: string): Promise<number> {
  return page.locator('[data-testid="message-row"]').filter({ hasText: text }).count();
}

// ---------------------------------------------------------------------------
// 管理面（仅测试环境）：确保首个用户是 op5，便于建频道
// ---------------------------------------------------------------------------

const ADMIN_BASE = 'http://127.0.0.1:18081';
const ADMIN_TOKEN = 'qa-admin-token';

async function adminFetch(path: string, init: RequestInit = {}) {
  return fetch(`${ADMIN_BASE}${path}`, {
    ...init,
    headers: {
      'X-Admin-Token': ADMIN_TOKEN,
      'Content-Type': 'application/json',
      ...(init.headers ?? {}),
    },
  });
}

export async function ensureOp5(loginId: string) {
  const res = await adminFetch('/api/users?limit=200');
  if (!res.ok) throw new Error(`admin list users failed: ${res.status}`);
  const list = (await res.json()) as Array<{ login_id: string; op: string }>;
  for (const u of list) {
    if (u.op === 'op5' && u.login_id !== loginId) {
      await adminFetch(`/api/users/${encodeURIComponent(u.login_id)}/op`, {
        method: 'POST',
        body: JSON.stringify({ op: 'op4' }),
      });
    }
  }
  const promote = await adminFetch(`/api/users/${encodeURIComponent(loginId)}/op`, {
    method: 'POST',
    body: JSON.stringify({ op: 'op5' }),
  });
  if (!promote.ok) throw new Error(`ensureOp5(${loginId}) failed: ${promote.status} ${await promote.text()}`);
}

// ---------------------------------------------------------------------------
// 独立文字频道（避免多个用例串扰同一个 MLS group）
// ---------------------------------------------------------------------------

export async function setupIsolatedChannel(
  humans: Human[],
  prefix = 'iso',
): Promise<{ id: string; name: string }> {
  const name = `${prefix}-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6)}`;
  await createNodeViaUI(humans[0].page, { name, kind: 'text' });
  await waitForNode(humans[0].page, name);
  for (const h of humans.slice(1)) await waitForNode(h.page, name, { timeout: 35_000 });

  const chId = await humans[0].page.evaluate(n => {
    const st = (window as any).__kaede_debug__.store.getState();
    return st.nodes.find((x: any) => x.name === n)?.id as string | undefined;
  }, name);
  if (!chId) throw new Error(`setupIsolatedChannel: node id not found for ${name}`);

  await clickChannel(humans[0].page, name);
  await expect(humans[0].page.getByRole('heading', { name: `#${name}` })).toBeVisible({ timeout: 20_000 });
  await waitChannelGroup(humans[0].page, chId);

  for (const h of humans.slice(1)) {
    await clickChannel(h.page, name);
    await expect(h.page.getByRole('heading', { name: `#${name}` })).toBeVisible({ timeout: 20_000 });
    await waitChannelGroup(h.page, chId);
  }
  return { id: chId, name };
}

/** 收/发两侧就绪屏障：发一条 sentinel，确认每个用户都能收到。 */
export async function syncRoom(humans: Human[], opts: { timeoutMs?: number } = {}): Promise<string> {
  const timeoutMs = opts.timeoutMs ?? 30_000;
  const tag = `sync_${Date.now().toString(36)}_${Math.floor(Math.random() * 1e9).toString(36)}`;
  await sendMessageHuman(humans[0].page, tag);
  for (const h of humans) {
    await h.page.getByText(tag, { exact: false }).first().waitFor({ state: 'visible', timeout: timeoutMs });
  }
  await humans[0].page.waitForTimeout(800);
  return tag;
}
