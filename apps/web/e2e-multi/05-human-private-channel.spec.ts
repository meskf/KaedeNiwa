import { test, expect } from '@playwright/test';
import {
  spawnUsers,
  closeUsers,
  sendMessage,
  syncRoom,
  createNodeViaUI,
  waitForNode,
  nodeByName,
} from './lib/fixtures';
import { setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

test.describe.configure({ mode: 'serial' });

test.describe('HUMAN PRIVATE CHANNEL', () => {
  test('create private channel, password prompt, member join, isolation from non-members', async ({ browser }) => {
    test.setTimeout(300_000);
    const users = await spawnUsers(browser, 3, 'pv');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [owner, member, outsider] = users;
      await setupIsolatedChannel(users, 'iso');

      const t = tag();
      const channel = `secret-${t}`;
      const password = `pw-${t}-123`;

      // owner 通过 UI 创建私密频道
      await createNodeViaUI(owner.page, { name: channel, kind: 'private', password });

      // 其他用户通过 10s 轮询看到节点
      await waitForNode(member.page, channel, { timeout: 25_000 });
      await waitForNode(outsider.page, channel, { timeout: 25_000 });

      // member 点击 → 密码弹窗 → 错误密码 → 正确密码
      await nodeByName(member.page, channel).click();
      const prompt = member.page.getByTestId('password-prompt');
      await expect(prompt).toBeVisible({ timeout: 15_000 });

      const pwInput = member.page.getByTestId('password-prompt-input');
      await pwInput.click();
      await member.page.keyboard.type('wrong-password', { delay: 20 });
      await member.page.getByTestId('password-prompt-submit').click();
      await expect(member.page.getByTestId('password-prompt-error')).toBeVisible({ timeout: 15_000 });

      await pwInput.click();
      await pwInput.press('Control+A');
      await member.page.keyboard.type(password, { delay: 20 });
      await member.page.getByTestId('password-prompt-submit').click();
      await expect(prompt).toBeHidden({ timeout: 30_000 });

      // 先进者建组，发一条（owner 尚未加入，不要求能解历史）
      const pre = `pre-join-${t}`;
      await sendMessage(member.page, pre);
      await expect(member.page.getByText(pre, { exact: false }).first()).toBeVisible({ timeout: 20_000 });

      // owner 点击同一频道 → 密码 → 加入（member 是频道 authority，自动 Welcome）
      await nodeByName(owner.page, channel).click();
      await expect(owner.page.getByTestId('password-prompt')).toBeVisible({ timeout: 15_000 });
      const ownerPw = owner.page.getByTestId('password-prompt-input');
      await ownerPw.click();
      await owner.page.keyboard.type(password, { delay: 20 });
      await owner.page.getByTestId('password-prompt-submit').click();
      await expect(owner.page.getByTestId('password-prompt')).toBeHidden({ timeout: 40_000 });

      // 加入后双方能实时互发
      const after = `after-join-${t}`;
      await sendMessage(owner.page, after);
      await expect(member.page.getByText(after, { exact: false }).first()).toBeVisible({ timeout: 25_000 });

      const reply = `reply-${t}`;
      await sendMessage(member.page, reply);
      await expect(owner.page.getByText(reply, { exact: false }).first()).toBeVisible({ timeout: 25_000 });

      // 外来者：点频道 → 弹窗 → 取消 → 看不到任何私密消息
      await nodeByName(outsider.page, channel).click();
      await expect(outsider.page.getByTestId('password-prompt')).toBeVisible({ timeout: 15_000 });
      await outsider.page.getByTestId('password-prompt-cancel').click();
      await expect(outsider.page.getByTestId('password-prompt')).toBeHidden({ timeout: 10_000 });

      for (const secret of [pre, after, reply]) {
        expect(
          await outsider.page.getByText(secret, { exact: false }).count(),
          `私密频道内容泄露给非成员: ${secret}`,
        ).toBe(0);
      }

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});

