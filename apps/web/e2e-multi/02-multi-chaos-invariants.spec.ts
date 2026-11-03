import { test, expect, Page } from '@playwright/test';
import { spawnUsers, sendMessage, closeUsers, syncRoom, User } from './lib/fixtures';
import { setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector, ErrorCollector } from './lib/observe';

// 混沌时序压力大，跨 worker 并行会互相踩。
// 3 个 scenario 串行跑，否则 A 离线 / B 离线 / C 离线同时发生，
// 服务端压力峰值本身就成了测试 artifact。
test.describe.configure({ mode: 'serial' });

function tag(prefix: string): string {
  return `m_${prefix}_${Date.now().toString(36)}_${Math.floor(Math.random() * 1e9).toString(36)}`;
}
async function countOn(page: Page, text: string): Promise<number> {
  return page.getByText(text, { exact: false }).count();
}
function failOnErr(cols: ErrorCollector[], label: string) {
  const noise = cols.flatMap(c => c.getNoise());
  const all = cols.flatMap(c => c.getFiltered());
  if (noise.length > 0) console.log(`[${label}] ignored ${noise.length} expected network noise`);
  if (all.length > 0) {
    console.log(`\n===== ${label} errors (${all.length}) =====`);
    for (const e of all.slice(0, 20)) console.log(e);
  }
  expect(all, `${label}: unexpected errors`).toEqual([]);
}

// WS 重连 + MLS 重新握手 + history backfill + 服务端 broadcast，全部走完需要时间。
const RECOVER_WAIT_MS = 25_000;

test.describe('CHAOS INVARIANTS', () => {
  test('01 offline-recovery-no-loss', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 3, 'c1');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b, c] = users;
      await setupIsolatedChannel(users, 'iso');

      await a.ctx.setOffline(true);
      await a.page.waitForTimeout(500); // 让 A 的 WS 真正断开

      const texts: string[] = [];
      for (const u of [b, c]) {
        for (let i = 0; i < 2; i++) {
          const t = tag('c1');
          texts.push(t);
          await sendMessage(u.page, t);
        }
      }
      await c.page.waitForTimeout(2_000); // B/C 消息在服务端落库 + 准备转发
      await a.ctx.setOffline(false);
      await a.page.waitForTimeout(RECOVER_WAIT_MS);

      for (const t of texts) {
        const cA = await countOn(a.page, t);
        expect(cA, `offline-loss: A missing or duplicated "${t}" (count=${cA})`).toBe(1);
      }
      for (const u of [b, c]) {
        for (const t of texts) {
          const cu = await countOn(u.page, t);
          expect(cu, `sender-side duplicate on ${u.name}: "${t}" (count=${cu})`).toBe(1);
        }
      }
      failOnErr(cols, 'offline-recovery');
    } finally {
      await closeUsers(users);
    }
  });

  test('02 offline-send-no-ghost', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'c2');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      // [diag] 打开 chat.ts 的 fallback 调试输出
      for (const u of users) {
        await u.page.evaluate(() => { (window as any).__KAEDE_QA_DEBUG__ = true; });
        u.page.on('console', (m) => {
          const t = m.text();
          if (t.startsWith('[qa-p1-debug]')) console.log(`[${u.name}] ${t}`);
        });
      }
      await setupIsolatedChannel(users, 'iso');
      // 补测反向：B→A 也必须通
      const probe = tag('c2p');
      await sendMessage(b.page, probe);
      await expect(a.page.getByText(probe, { exact: false }).first()).toBeVisible({ timeout: 25_000 });

      await a.ctx.setOffline(true);
      await a.page.waitForTimeout(500);

      const m = tag('c2m');
      await sendMessage(a.page, m).catch(() => {});
      await a.page.waitForTimeout(2_000);
      await a.ctx.setOffline(false);
      await a.page.waitForTimeout(RECOVER_WAIT_MS);

      const aCount = await countOn(a.page, m);
      const bCount = await countOn(b.page, m);
      console.log(`[02] aCount=${aCount} bCount=${bCount}`);

      // [diag] 区分 2 条 M 的容器：messages vs pending-msg
      const diag = await a.page.evaluate((text) => {
        const msgs = Array.from(document.querySelectorAll('[data-msg-sender]'))
          .filter(el => el.textContent?.includes(text))
          .map(el => ({
            ts: el.getAttribute('data-msg-ts'),
            sender: el.getAttribute('data-msg-sender'),
          }));
        const pending = Array.from(document.querySelectorAll('[data-testid="pending-msg"]'))
          .filter(el => el.textContent?.includes(text))
          .map(el => ({
            status: el.getAttribute('data-status'),
            textPreview: el.textContent?.slice(0, 60),
          }));
        const totalMsgs = document.querySelectorAll('[data-msg-sender]').length;
        const totalPending = document.querySelectorAll('[data-testid="pending-msg"]').length;
        return { msgs, pending, totalMsgs, totalPending };
      }, m);
      console.log(`[diag] A DOM: msgs_in_messages=${diag.msgs.length} pending_nodes=${diag.pending.length} totalMsgs=${diag.totalMsgs} totalPending=${diag.totalPending}`);
      console.log(`[diag] msgs=${JSON.stringify(diag.msgs)}`);
      console.log(`[diag] pending=${JSON.stringify(diag.pending)}`);

      expect(aCount, `A duplicate: count=${aCount}`).toBeLessThanOrEqual(1);
      expect(bCount, `B duplicate: count=${bCount}`).toBeLessThanOrEqual(1);
      if (aCount === 1) {
        expect(bCount, `ghost: A shows M but B does not (aCount=1, bCount=0)`).toBe(1);
      }
      failOnErr(cols, 'offline-send');
    } finally {
      await closeUsers(users);
    }
  });

  test('03 multi-user-jitter-consistency', async ({ browser }) => {
    test.setTimeout(240_000);
    const users = await spawnUsers(browser, 3, 'c3');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b, c] = users;
      await setupIsolatedChannel(users, 'iso');

      // C 全程在线发的 3 条：先确认 A/B 全部收到，再开始离线（这是测试前提，不是断言目标）
      const cTexts: string[] = [];
      for (let i = 0; i < 3; i++) {
        const t = tag('c3c');
        cTexts.push(t);
        await sendMessage(c.page, t);
      }
      for (const t of cTexts) {
        await a.page.getByText(t, { exact: false }).first().waitFor({ state: 'visible', timeout: 25_000 });
        await b.page.getByText(t, { exact: false }).first().waitFor({ state: 'visible', timeout: 25_000 });
      }

      // A 离线 2s
      await a.ctx.setOffline(true);
      await a.page.waitForTimeout(500);
      const aTexts: string[] = [];
      for (let i = 0; i < 2; i++) {
        const t = tag('c3a');
        aTexts.push(t);
        await sendMessage(a.page, t).catch(() => {});
      }
      await a.page.waitForTimeout(2_000);
      await a.ctx.setOffline(false);

      // B 离线 3s
      await b.ctx.setOffline(true);
      await b.page.waitForTimeout(500);
      const bTexts: string[] = [];
      for (let i = 0; i < 2; i++) {
        const t = tag('c3b');
        bTexts.push(t);
        await sendMessage(b.page, t).catch(() => {});
      }
      await b.page.waitForTimeout(3_000);
      await b.ctx.setOffline(false);

      await a.page.waitForTimeout(RECOVER_WAIT_MS);
      await b.page.waitForTimeout(2_000);
      await c.page.waitForTimeout(2_000);

      const all = [...cTexts, ...aTexts, ...bTexts];
      for (const t of all) {
        const counts = await Promise.all([a, b, c].map(u => countOn(u.page, t)));
        const lbl = cTexts.includes(t) ? 'C-sent' : aTexts.includes(t) ? 'A-offline-sent' : 'B-offline-sent';
        console.log(`[03] "${t}" (${lbl}) => A=${counts[0]} B=${counts[1]} C=${counts[2]}`);
        for (let i = 0; i < 3; i++) {
          expect(counts[i], `duplicate on u${i} for "${t}" (count=${counts[i]})`).toBeLessThanOrEqual(1);
        }
        if (cTexts.includes(t)) {
          expect(counts, `C-sent "${t}" inconsistent: ${counts}`).toEqual([1, 1, 1]);
        } else {
          const allZero = counts.every(c => c === 0);
          const allOne  = counts.every(c => c === 1);
          expect(allZero || allOne, `"${t}" consistency broken: A=${counts[0]} B=${counts[1]} C=${counts[2]}`).toBe(true);
        }
      }
      failOnErr(cols, 'jitter');
    } finally {
      await closeUsers(users);
    }
  });

  test.fixme('04 ws-send-jitter', async () => {});
});

