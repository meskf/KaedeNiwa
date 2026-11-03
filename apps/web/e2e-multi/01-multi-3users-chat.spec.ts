import { test, expect } from '@playwright/test';
import { spawnUsers, sendMessage, closeUsers, setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

test.describe('MULTI-USER CHAT', () => {
  test('3 users interleaved send, all see all, zero console errors', async ({ browser }) => {
    test.setTimeout(180_000);

    const users = await spawnUsers(browser, 3, 'multi');
    await setupIsolatedChannel(users, 'm3');
    const collectors = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));

    // 拦截 console.error 抓真实 args
    for (const u of users) {
      await u.page.evaluate(() => {
        const w = window as any;
        w.__kaede_dup_key_warnings = [];
        const orig = console.error;
        console.error = function (...args: any[]) {
          if (typeof args[0] === 'string' && args[0].includes('same key')) {
            w.__kaede_dup_key_warnings.push({ args: args.slice(0, 4), stack: new Error().stack });
          }
          return (orig as any).apply(console, args);
        };
      });
    }

    try {
      const msgsPerUser = 5;
      const allMsgs: string[] = [];
      const epoch = Date.now().toString(36);

      for (let round = 0; round < msgsPerUser; round++) {
        for (let i = 0; i < users.length; i++) {
          const text = `M${i}R${round}_${epoch}`;
          allMsgs.push(text);
          await sendMessage(users[i].page, text);
        }
      }

      await users[0].page.waitForTimeout(3000);

      for (let i = 0; i < users.length; i++) {
        for (const text of allMsgs) {
          await expect(
            users[i].page.getByText(text, { exact: false }).first(),
            `user${i} should see ${text}`
          ).toBeVisible({ timeout: 25_000 });
          const count = await users[i].page
            .locator('[data-testid="message-row"]')
            .filter({ hasText: text })
            .count();
          expect(count, `user${i} duplicate of ${text} (count=${count})`).toBe(1);
        }
      }

      // DOM hist per user
      console.log(`\n===== DOM COUNT HISTOGRAM (getByText matches per message) =====`);
      for (let i = 0; i < users.length; i++) {
        const hist = new Map<number, number>();
        for (const text of allMsgs) {
          const c = await users[i].page.getByText(text, { exact: false }).count();
          hist.set(c, (hist.get(c) ?? 0) + 1);
        }
        const entries = Array.from(hist.entries()).sort((a, b) => a[0] - b[0]);
        console.log(`u${i}: ${entries.map(([k, v]) => `count=${k}×${v}msgs`).join(', ')}`);
      }

      // React key warnings
      const captured = await users[0].page.evaluate(() => (window as any).__kaede_dup_key_warnings || []);
      console.log(`\n===== REACT KEY WARNINGS (${captured.length}) =====`);
      const seen = new Set<string>();
      let printed = 0;
      for (const w of captured) {
        const args = w.args || [];
        // args = [format, keyValue, ...rest]
        const keyDisplay = String(args[1] ?? args[0] ?? '');
        if (seen.has(keyDisplay)) continue;
        seen.add(keyDisplay);
        printed++;
        if (printed > 10) break;
        console.log(`\n[KEY=${keyDisplay}]`);
        const argsJson = JSON.stringify(args).slice(0, 500);
        console.log(`  args: ${argsJson}`);
        if (w.stack) {
          const lines = String(w.stack).split('\n').slice(1, 7);
          for (const l of lines) console.log(`  | ${l.trim().slice(0, 200)}`);
        }
      }
      if (seen.size > printed) console.log(`\n  ... ${seen.size - printed} more unique keys`);

      const all = collectors.flatMap(c => c.events());
      const errs = collectors.flatMap(c => c.get());

      if (errs.length > 0) {
        const groups = new Map<string, { count: number; sample: any }>();
        for (const e of all) {
          const sig =
            e.kind === 'http' ? `http ${e.detail.status} ${e.detail.method} ${e.detail.url.replace(/[a-f0-9-]{8,}/g, '<id>')}`
            : e.kind === 'console' ? `console: ${(e.detail.text || '').slice(0, 200)}`
            : e.kind === 'reqfail' ? `reqfail: ${e.detail.method} ${e.detail.url.replace(/[a-f0-9-]{8,}/g, '<id>')} ${e.detail.failure}`
            : `${e.kind}: ${e.detail.message}`;
          if (!groups.has(sig)) groups.set(sig, { count: 0, sample: e.detail });
          groups.get(sig)!.count++;
        }
        console.log(`\n===== ERROR GROUPS (${errs.length} total / ${groups.size} unique) =====`);
        for (const [k, v] of groups) {
          console.log(`\n[${v.count}x] ${k}`);
        }
      }

      expect(errs, `got ${errs.length} errors — see ERROR GROUPS above`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});
