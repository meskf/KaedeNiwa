import { test, expect } from '@playwright/test';
import {
  spawnHumans,
  closeHumans,
  setupIsolatedChannel,
  sendMessageHuman,
  messageRowByText,
  countMessageRows,
  humanType,
  humanPause,
  pasteAndSend,
  hardErrors,
} from '../lib/core';

test.describe('真人聊天：输入体验 / 富文本 / 表情 / 搜索 / 附件 / 连发', () => {
  test('对方正在输入 → 完整送达（中文长句）', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'chat-typing');
    try {
      await setupIsolatedChannel(humans, 'talk');
      const [h0, h1] = humans;

      const first = '今天先看这段';
      const rest = '共享画面，帮我确认一下卡不卡';
      await h0.page.getByTestId('message-input').click();
      await humanType(h0.page, first, { delay: 55, pauses: false });

      const typing = h1.page.getByTestId('typing-indicator');
      await expect(typing).toBeVisible({ timeout: 15_000 });
      await expect(typing).toContainText(/正在输入/);

      await humanType(h0.page, rest, { delay: 45 });
      await humanPause(h0.page, 200, 400);
      await h0.page.keyboard.press('Enter');

      const row = messageRowByText(h1.page, first);
      await expect(row).toBeVisible({ timeout: 30_000 });
      await expect(row).toContainText(rest);
      await expect(typing).toHaveCount(0);

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('多行 + emoji + Markdown 粗体/代码，跨端渲染一致', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'chat-rich');
    try {
      await setupIsolatedChannel(humans, 'rich');
      const [h0, h1] = humans;

      const box = h0.page.getByTestId('message-input');
      await box.click();
      await humanType(h0.page, '第一行');
      await h0.page.keyboard.press('Shift+Enter');
      await humanType(h0.page, '第二行');
      await h0.page.keyboard.insertText('🎉');
      await humanPause(h0.page, 250, 450);
      await h0.page.keyboard.press('Enter');

      const multiline = messageRowByText(h1.page, '第一行');
      await expect(multiline).toBeVisible({ timeout: 30_000 });
      await expect(multiline).toContainText('第二行');
      await expect(multiline).toContainText('🎉');

      await sendMessageHuman(h0.page, '**重点** 和 `code` 都要正确显示');
      const rich = messageRowByText(h1.page, '重点');
      await expect(rich).toBeVisible({ timeout: 30_000 });
      await expect(rich.locator('strong')).toHaveText('重点');
      await expect(rich.locator('code')).toHaveText('code');

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('右键消息 → 快速表情回应 → 两边同步 → 再点收回', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'chat-react');
    try {
      await setupIsolatedChannel(humans, 'react');
      const [h0, h1] = humans;
      const token = '表情消息-' + Date.now().toString(36);
      await sendMessageHuman(h0.page, token);
      await expect(messageRowByText(h1.page, token)).toBeVisible({ timeout: 30_000 });

      // 真人在对方消息上右键，选一个表情
      await messageRowByText(h1.page, token).click({ button: 'right' });
      await expect(h1.page.getByTestId('message-context-menu')).toBeVisible({ timeout: 10_000 });
      await h1.page.getByTestId('quick-react-👍').click();

      const pill = h0.page.getByTestId('reaction-pill-👍');
      await expect(pill).toBeVisible({ timeout: 20_000 });
      await expect(pill).toHaveAttribute('data-count', '1');
      await expect(h1.page.getByTestId('reaction-pill-👍')).toBeVisible({ timeout: 20_000 });

      // 本人再点一次同一表情 = 收回
      await h1.page.getByTestId('reaction-pill-👍').click();
      await expect(h0.page.getByTestId('reaction-pill-👍')).toHaveCount(0, { timeout: 20_000 });
      await expect(h1.page.getByTestId('reaction-pill-👍')).toHaveCount(0, { timeout: 20_000 });

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('搜索：真人点放大镜、输入关键字、命中并跳转', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'chat-search');
    try {
      await setupIsolatedChannel(humans, 'search');
      const [h0, h1] = humans;
      const token = 'needle' + Date.now().toString(36);
      await sendMessageHuman(h0.page, '这里埋一个搜索标记 ' + token);
      await expect(messageRowByText(h0.page, token)).toBeVisible({ timeout: 20_000 });

      await h0.page.getByTestId('search-trigger').click();
      const input = h0.page.getByTestId('search-input');
      await expect(input).toBeVisible({ timeout: 10_000 });
      await input.click();
      await humanType(h0.page, token, { delay: 25, pauses: false });

      const first = h0.page.getByTestId('search-result-0');
      await expect(first).toBeVisible({ timeout: 15_000 });
      await expect(first).toContainText(token);
      await first.click();
      await expect(input).toHaveCount(0, { timeout: 10_000 });

      // 搜索无结果时不崩：输入乱码前缀
      await h0.page.getByTestId('search-trigger').click();
      await expect(h0.page.getByTestId('search-input')).toBeVisible();
      await h0.page.getByTestId('search-input').click();
      await humanType(h0.page, 'zzz-no-such-' + Date.now().toString(36), { delay: 20, pauses: false });
      await expect(h0.page.getByTestId('search-result-0')).toHaveCount(0, { timeout: 10_000 });
      await h0.page.keyboard.press('Escape');

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('附件：选文件 → 预览 → 带说明发送 → 对端可下载', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'chat-file');
    try {
      await setupIsolatedChannel(humans, 'file');
      const [h0, h1] = humans;
      const caption = '附件说明-' + Date.now().toString(36);

      await h0.page.getByTestId('attachment-file-input').setInputFiles({
        name: 'human-note.txt',
        mimeType: 'text/plain',
        buffer: Buffer.from('真人自动化测试附件内容 ' + caption),
      });
      await expect(h0.page.getByTestId('message-input')).toBeVisible();

      await h0.page.getByTestId('message-input').click();
      await humanType(h0.page, caption, { delay: 25 });
      await humanPause(h0.page, 300, 500);
      await h0.page.keyboard.press('Enter');

      const row = messageRowByText(h1.page, caption);
      await expect(row).toBeVisible({ timeout: 40_000 });
      await expect(row).toContainText('human-note.txt');

      const downloadPromise = h1.page.waitForEvent('download', { timeout: 25_000 });
      await row.getByRole('button', { name: /human-note.txt/ }).click();
      const download = await downloadPromise;
      expect(download.suggestedFilename()).toBe('human-note.txt');

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });

  test('连发 15 条：全部送达、顺序正确、无重复无丢失', async ({ browser }) => {
    const humans = await spawnHumans(browser, 2, 'chat-burst');
    try {
      await setupIsolatedChannel(humans, 'burst');
      const [h0, h1] = humans;
      const base = 'burst' + Date.now().toString(36);

      // 用 #n# 包裹序号，避免 "-1" 命中 "-10/-11" 这类子串
      const mark = (i: number) => `${base}#${i}#`;
      for (let i = 0; i < 15; i++) {
        await pasteAndSend(h0.page, mark(i));
        await h0.page.waitForTimeout(80 + Math.round(Math.random() * 180));
      }

      await expect(messageRowByText(h1.page, mark(14))).toBeVisible({ timeout: 45_000 });
      for (let i = 0; i < 15; i++) {
        expect(await countMessageRows(h1.page, mark(i))).toBe(1);
      }

      // 顺序：最后一条在 DOM 里应位于最后
      const texts = await h1.page.getByTestId('message-row').allInnerTexts();
      const indices = texts
        .map(t => {
          const m = t.match(new RegExp(`${base}#(\\d+)#`));
          return m ? Number(m[1]) : null;
        })
        .filter((n): n is number => n !== null);
      expect(indices).toEqual([...indices].sort((a, b) => a - b));

      expect(hardErrors(h0)).toEqual([]);
      expect(hardErrors(h1)).toEqual([]);
    } finally {
      await closeHumans(humans);
    }
  });
});
