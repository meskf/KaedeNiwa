import { setupIsolatedChannel } from './lib/fixtures';
import { test, expect, Browser } from '@playwright/test';
import {
  spawnUsers,
  closeUsers,
  sendMessage,
  syncRoom,
  switchView,
  createNodeViaUI,
  waitForNode,
  nodeByName,
  countMessageRows,
  loginExisting,
  login,
  randName,
  waitForChannelReady,
  User,
} from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

test.describe.configure({ mode: 'serial' });

async function makeUser(browser: Browser, name: string, register = true): Promise<User> {
  const ctx = await browser.newContext({ ignoreHTTPSErrors: true, permissions: ['camera', 'microphone'] });
  const page = await ctx.newPage();
  await login(page, name, undefined, { register });
  return { ctx, page, name };
}

test.describe('CHAOS / REAL-WORLD', () => {
  test('01 same-account second login takes over the session', async ({ browser }) => {
    test.setTimeout(240_000);
    const name = randName('takeover');
    const a = await makeUser(browser, name, true);
    const b = await makeUser(browser, name, false);
    const observer = await makeUser(browser, randName('obs'), true);
    const cols = [
      attachErrorCollector(a.page, 'old'),
      attachErrorCollector(b.page, 'new'),
      attachErrorCollector(observer.page, 'obs'),
    ];
    try {
      // 新会话与观察者都必须完成频道 group 建立后再发消息；
      // 顶替后旧会话被踢，新会话可能触发“权威离线接管”，需要几秒。
      await waitForChannelReady(b.page, 90_000);
      await waitForChannelReady(observer.page, 90_000);
      const probe = `takeover-probe-${tag()}`;
      await sendMessage(observer.page, probe);
      await expect(b.page.getByText(probe, { exact: false }).first()).toBeVisible({ timeout: 25_000 });

      // 旧会话必须收不到新消息（服务端已断开它的 WS）
      await a.page.waitForTimeout(8_000);
      expect(
        await a.page.getByText(probe, { exact: false }).count(),
        '旧会话仍然收到消息（未被顶替）',
      ).toBe(0);

      // 新会话可以正常发送，旧会话也看不到
      const sent = `takeover-new-${tag()}`;
      await sendMessage(b.page, sent);
      await expect(observer.page.getByText(sent, { exact: false }).first()).toBeVisible({ timeout: 25_000 });
      expect(await a.page.getByText(sent, { exact: false }).count()).toBe(0);

      // 旧会话应回到登录页（自动重连用旧 token 会被拒绝）
      await expect(a.page.getByTestId('login-id')).toBeVisible({ timeout: 90_000 });

      // 顶替过程中旧会话必然出现 401（历史/节点请求）——这正是预期语义。
      const errs = cols.flatMap(c => c.getFiltered())
        .filter(e => !/Unauthenticated|unauthorized|session superseded|http 401/i.test(e));
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      for (const c of [a.ctx, b.ctx, observer.ctx]) await c.close().catch(() => {});
    }
  });

  test('02 three users join a brand-new channel simultaneously, no split', async ({ browser }) => {
    test.setTimeout(240_000);
    const users = await spawnUsers(browser, 3, 'race');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      await setupIsolatedChannel(users, 'chaos');
      const t = tag();
      const channel = `race-${t}`;
      await createNodeViaUI(users[0].page, { name: channel, kind: 'text' });
      await waitForNode(users[0].page, channel);
      for (const u of users.slice(1)) await waitForNode(u.page, channel, { timeout: 25_000 });

      // 三人同时点击同一频道（最坏时序：三份 join + 三份 KP 同时提交）
      await Promise.all(users.map(u => nodeByName(u.page, channel).click()));
      for (const u of users) {
        await expect(u.page.getByRole('heading', { name: `#${channel}` })).toBeVisible({ timeout: 25_000 });
      }
      const chId = await users[0].page.evaluate((n) => {
        const st = (window as any).__kaede_debug__.store.getState();
        return st.nodes.find((x: any) => x.name === n)?.id as string | undefined;
      }, channel);
      expect(chId, 'race channel id not found').toBeTruthy();
      // 必须等三人都在同一个 group 里再发消息；只等标题可见会在 welcome 未完成时误判“丢消息”
      for (const u of users) {
        await u.page.waitForFunction(async (id) => {
          const dbg = (window as any).__kaede_debug__;
          if (!dbg) return false;
          const st = await dbg.channelState(id);
          return !!(st && st.has_group);
        }, chId, { timeout: 60_000 });
      }
      await users[0].page.waitForTimeout(2_000);

      const texts: string[] = [];
      for (let i = 0; i < users.length; i++) {
        const text = `race-msg-${t}-${i}`;
        texts.push(text);
        await sendMessage(users[i].page, text);
      }
      for (const u of users) {
        for (const text of texts) {
          await expect(
            u.page.getByText(text, { exact: false }).first(),
            `${u.name} missing ${text}`,
          ).toBeVisible({ timeout: 25_000 });
          expect(await countMessageRows(u.page, text)).toBe(1);
        }
      }

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('03 long outage: heartbeat detects dead link, auto-reconnect, no message loss', async ({ browser }) => {
    test.setTimeout(300_000);
    const users = await spawnUsers(browser, 2, 'wsf');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'chaos');

      const joinedCount = () => a.page.evaluate(
        () => ((window as any).__kaede_debug__.events.list('Joined') as any[]).length,
      );
      const before = await joinedCount();
      const t = tag();

      // A 断网 35s（超过 20s ping / 30s timeout 心跳窗口），B 正常发消息。
      await a.ctx.setOffline(true);
      const during: string[] = [];
      for (let i = 0; i < 3; i++) {
        const text = `outage-${t}-${i}`;
        during.push(text);
        await sendMessage(b.page, text);
      }
      await a.page.waitForTimeout(35_000);
      await a.ctx.setOffline(false);

      // 心跳应检测到断链并触发自动重连（Joined 事件计数增加）
      await expect.poll(joinedCount, { timeout: 120_000 }).toBeGreaterThan(before);

      // 断网期间 B 发的消息不能丢
      for (const text of during) {
        await expect(a.page.getByText(text, { exact: false }).first()).toBeVisible({ timeout: 30_000 });
      }

      // 恢复后 A 能正常发送，B 能收到
      const after = `resume-${t}`;
      await sendMessage(a.page, after);
      await expect(b.page.getByText(after, { exact: false }).first()).toBeVisible({ timeout: 30_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});


