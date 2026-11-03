import { Browser, BrowserContext, Page, expect } from '@playwright/test';

export const PW = 'password123';
export const randName = (p: string) => `${p}-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6)}`;

/**
 * 真人登录：所有输入走 keyboard.type（触发 keydown/keypress/input），
 * 不用 fill（fill 是直接 setValue，绕过键盘事件，等于跳过了用户实际敲字的延迟和事件链）。
 *
 * register 默认 true（测试新账号）；已存在账号用 register:false（登录而非注册）。
 */
export async function login(page: Page, loginId: string, password = PW, opts: { register?: boolean } = {}) {
  const register = opts.register !== false;
  await page.goto('/');
  const idBox = page.getByTestId('login-id');
  await idBox.waitFor({ state: 'visible', timeout: 30_000 });
  await idBox.click();
  await page.keyboard.type(loginId, { delay: 20 });

  const pwBox = page.getByTestId('login-password');
  await pwBox.click();
  await page.keyboard.type(password, { delay: 20 });

  // 默认是登录模式；注册需要点 toggle。
  const reg = page.getByTestId('login-toggle-register');
  if (register) {
    if (await reg.isVisible()) await reg.click();
  }

  await page.getByTestId('login-submit').click();

  // 注册可能被限流（backend: ip:{ip}:register, 0.2 tokens/s cap 10）
  // 若 5s 内登录页还在，读错误文本抛出，而不是傻等 message-input
  const inputOrErr = await Promise.race([
    page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 15_000 }).then(() => 'ok').catch(() => 'timeout'),
    page.getByText(/internal error|rate limit|login_id_taken|Invalid account|already taken/i).first().waitFor({ state: 'visible', timeout: 15_000 }).then(() => 'err').catch(() => 'timeout'),
  ]);
  if (inputOrErr === 'err') {
    const msg = await page.getByText(/internal error|rate limit|login_id_taken|Invalid account|already taken/i).first().textContent();
    throw new Error(`login/register failed: ${msg}`);
  }
  if (inputOrErr === 'timeout') {
    throw new Error(`login/register timeout (rate limit?): user=${loginId}`);
  }
  await page.waitForFunction(() => !!(window as any).__kaede_debug__, null, { timeout: 25_000 });
}

/** 已注册账号登录（不点注册 toggle）。 */
export async function loginExisting(page: Page, loginId: string, password = PW) {
  await login(page, loginId, password, { register: false });
}

export async function sendMessage(page: Page, text: string) {
  const box = page.getByTestId('message-input');
  await box.click();
  await page.keyboard.type(text, { delay: 15 });
  await page.keyboard.press('Enter');
}

export type User = { ctx: BrowserContext; page: Page; name: string };

export async function spawnUsers(browser: Browser, count: number, prefix = 'u'): Promise<User[]> {
  const users: User[] = [];
  for (let i = 0; i < count; i++) {
    const name = randName(`${prefix}${i}`);
    let lastErr: Error | null = null;
    for (let attempt = 0; attempt < 3; attempt++) {
      const ctx = await browser.newContext({ ignoreHTTPSErrors: true, permissions: ['camera', 'microphone'] });
      const page = await ctx.newPage();
      try {
        await login(page, name);
        users.push({ ctx, page, name });
        lastErr = null;
        break;
      } catch (e) {
        lastErr = e as Error;
        await ctx.close().catch(() => {});
        if (attempt < 2) {
          console.log(`[spawnUsers] ${name} attempt ${attempt+1}/3 failed (${(e as Error).message?.slice(0,100)}), retry in 2s`);
          await new Promise(r => setTimeout(r, 2000));
        }
      }
    }
    if (lastErr) throw new Error(`spawnUsers: ${name} failed after 3 attempts: ${lastErr.message}`);
  }
  return users;
}

export async function closeUsers(users: User[]) {
  for (const u of users) { try { await u.ctx.close(); } catch {} }
}

/** 等待当前默认文字频道真正建立 MLS group（只读探测，不改变状态）。 */
export async function waitForChannelReady(page: Page, timeoutMs = 40_000) {
  await page.waitForFunction(async () => {
    const dbg = (window as any).__kaede_debug__;
    if (!dbg) return false;
    const list = await dbg.channels.list();
    const ch = list.find((c: any) => c.kind === 'text');
    if (!ch) return false;
    // channelState 是 async，必须 await，否则拿到的是 Promise（永远 undefined）
    const st = await dbg.channelState(ch.id);
    return !!(st && st.has_group);
  }, null, { timeout: timeoutMs });
}

// ---- 管理面编排（仅测试环境：127.0.0.1:18081 + 固定 token） ----
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

/**
 * 确保指定账号是 op5（服主）。
 * 若已有别的 op5，先降为 op4，再提升目标账号。
 *
 * 用途：e2e 数据库可能被前序用例写过，不能假设“本次第一个注册者一定是 op5”。
 */
export async function ensureOp5(loginId: string) {
  const res = await adminFetch('/api/users?limit=200');
  if (!res.ok) throw new Error(`admin list users failed: ${res.status}`);
  const list = await res.json() as Array<{ login_id: string; op: string }>;
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
  if (!promote.ok) {
    throw new Error(`ensureOp5(${loginId}) failed: ${promote.status} ${await promote.text()}`);
  }
}

// ---- 真人 UI 操作助手 ----

export async function switchView(page: Page, view: 'chat' | 'dms' | 'friends' | 'settings' | 'more') {
  await page.getByTestId(`nav-${view}`).click();
}

/** 频道树里按名字定位节点行（data-node-id 容器）。 */
export function nodeByName(page: Page, name: string) {
  return page.locator('[data-node-id]').filter({ hasText: name }).first();
}

export async function waitForNode(page: Page, name: string, opts: { timeout?: number; state?: 'visible' | 'detached' } = {}) {
  const node = nodeByName(page, name);
  await node.waitFor({ state: opts.state ?? 'visible', timeout: opts.timeout ?? 20_000 });
  return node;
}

/**
 * 通过 UI 新建节点：
 *   - 根节点：点侧栏 "+" 打开菜单
 *   - 子节点：右键父节点打开菜单
 */
export async function createNodeViaUI(
  page: Page,
  opts: { name: string; kind: 'folder' | 'text' | 'voice' | 'private'; password?: string; parent?: string },
) {
  if (opts.parent) {
    const parent = nodeByName(page, opts.parent);
    await parent.click({ button: 'right' });
  } else {
    await page.getByTestId('sidebar-create-node').click();
  }
  const menuTestId =
    opts.kind === 'private' ? 'ctx-create-private'
      : opts.kind === 'text' ? 'ctx-create-text'
        : opts.kind === 'voice' ? 'ctx-create-voice'
          : 'ctx-create-folder';
  await page.getByTestId(menuTestId).click();

  const nameInput = page.locator('input[placeholder="输入名称..."]');
  await nameInput.click();
  await page.keyboard.type(opts.name, { delay: 25 });

  if (opts.kind === 'private') {
    const pwInput = page.getByTestId('ctx-create-private-password');
    await pwInput.click();
    await page.keyboard.type(opts.password || PW, { delay: 20 });
  }
  await page.getByTestId('ctx-confirm').click();
}

/** 右键节点 → 重命名。 */
export async function renameNodeViaUI(page: Page, oldName: string, newName: string) {
  await nodeByName(page, oldName).click({ button: 'right' });
  await page.getByTestId('ctx-rename').click();
  const input = page.locator('input[placeholder="输入名称..."]');
  await input.click();
  await input.press('Control+A');
  await page.keyboard.type(newName, { delay: 25 });
  await page.getByTestId('ctx-confirm').click();
}

/** 右键节点 → 删除（接受原生 confirm）。 */
export async function deleteNodeViaUI(page: Page, name: string) {
  await nodeByName(page, name).click({ button: 'right' });
  page.once('dialog', d => d.accept());
  await page.getByTestId('ctx-delete').click();
}

export async function clickChannel(page: Page, name: string) {
  await nodeByName(page, name).click();
}

/** 在消息列表里找包含指定文本的 message-row。 */
export function messageRowByText(page: Page, text: string) {
  return page.locator('[data-testid="message-row"]').filter({ hasText: text }).first();
}

/** 统计某段文本在 message-row 中出现的次数（排除 pending / 其它 DOM 噪音）。 */
export async function countMessageRows(page: Page, text: string): Promise<number> {
  return page.locator('[data-testid="message-row"]').filter({ hasText: text }).count();
}

/**
 * 为当前测试创建独立文字频道，并让所有用户进入 + 建好 MLS group。
 *
 * 为什么必须隔离：所有用例共用 general 会在并行时互相触发“权威离线接管”，
 * 不同 worker 的 takeover 会把彼此的成员清出频道，导致消息丢失。
 * 独立频道让每个场景拥有自己的 group 生命周期，测试结果才可解释。
 */
export async function setupIsolatedChannel(
  users: User[],
  prefix = 'iso',
): Promise<{ id: string; name: string }> {
  const name = `${prefix}-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6)}`;
  await createNodeViaUI(users[0].page, { name, kind: 'text' });
  await waitForNode(users[0].page, name);
  for (const u of users.slice(1)) {
    await waitForNode(u.page, name, { timeout: 30_000 });
  }
  const chId = await users[0].page.evaluate((n) => {
    const st = (window as any).__kaede_debug__.store.getState();
    return st.nodes.find((x: any) => x.name === n)?.id as string | undefined;
  }, name);
  if (!chId) throw new Error(`setupIsolatedChannel: node id not found for ${name}`);

  // 创建者先进入并建好 group，再让其他人加入：
  // 同时点击会在“权威还没建群”时互相等待，触发超时接管，测试场景就不干净了。
  await clickChannel(users[0].page, name);
  await expect(users[0].page.getByRole('heading', { name: `#${name}` })).toBeVisible({ timeout: 20_000 });
  await users[0].page.waitForFunction(async (id) => {
    const dbg = (window as any).__kaede_debug__;
    if (!dbg) return false;
    const st = await dbg.channelState(id);
    return !!(st && st.has_group);
  }, chId, { timeout: 60_000 });

  for (const u of users.slice(1)) {
    await clickChannel(u.page, name);
    await expect(u.page.getByRole('heading', { name: `#${name}` })).toBeVisible({ timeout: 20_000 });
  }
  // 等每个客户端真正拿到频道 group（其余通过 Welcome 加入）
  for (const u of users.slice(1)) {
    await u.page.waitForFunction(async (id) => {
      const dbg = (window as any).__kaede_debug__;
      if (!dbg) return false;
      const st = await dbg.channelState(id);
      return !!(st && st.has_group);
    }, chId, { timeout: 60_000 });
  }
  await syncRoom(users);
  return { id: chId, name };
}
// ---- 混沌测试同步屏障 ----
// spawnUsers 只保证登录成功，不保证所有用户已进入同一 MLS 组，
// 也不保证用户能接收 WS 消息。混沌测试必须先用 sentinel 确认收/发两侧就绪，
// 否则"第一条消息丢失"会被误判为产品 bug。
export async function syncRoom(users: User[], opts: { timeoutMs?: number } = {}): Promise<string> {
  const timeoutMs = opts.timeoutMs ?? 30_000;
  const tag = `sync_${Date.now().toString(36)}_${Math.floor(Math.random() * 1e9).toString(36)}`;
  const sender = users[0];
  await sendMessage(sender.page, tag);
  for (let i = 0; i < users.length; i++) {
    try {
      await users[i].page.getByText(tag, { exact: false }).first().waitFor({
        state: 'visible',
        timeout: timeoutMs,
      });
    } catch (e) {
      throw new Error(
        `syncRoom failed: ${users[i].name} (index ${i}) cannot see sentinel within ${timeoutMs}ms. ` +
        `Sender=${sender.name}. 说明该用户未就绪接收 WS 消息 / 未入 MLS 组。`
      );
    }
  }
  await sender.page.waitForTimeout(1_000);
  return tag;
}
