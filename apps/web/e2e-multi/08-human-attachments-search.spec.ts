import { test, expect } from '@playwright/test';
import { spawnUsers, closeUsers, sendMessage, syncRoom, messageRowByText } from './lib/fixtures';
import { setupIsolatedChannel } from './lib/fixtures';
import { attachErrorCollector } from './lib/observe';

const tag = () => Date.now().toString(36).slice(-6);

// 1x1 PNG
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);

test.describe.configure({ mode: 'serial' });

test.describe('HUMAN ATTACHMENTS & SEARCH', () => {
  test('01 send text file and image attachment, peer receives and can download', async ({ browser }) => {
    test.setTimeout(240_000);
    const users = await spawnUsers(browser, 2, 'at');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'iso');

      const t = tag();
      const fileName = `note-${t}.txt`;
      const msg = `file-msg-${t}`;
      const fileContent = `hello-from-${t}`;
      const fileBuffer = Buffer.from(fileContent, 'utf8');

      // 走真实文件选择器（点击回形针 → filechooser）
      const chooserPromise = a.page.waitForEvent('filechooser');
      await a.page.getByTestId('attachment-picker').click();
      const chooser = await chooserPromise;
      await chooser.setFiles([{ name: fileName, mimeType: 'text/plain', buffer: fileBuffer }]);

      await a.page.getByTestId('message-input').click();
      await a.page.keyboard.type(msg, { delay: 20 });
      await a.page.keyboard.press('Enter');

      // 双方都看到消息与附件名
      await expect(b.page.getByText(msg, { exact: false }).first()).toBeVisible({ timeout: 25_000 });
      await expect(b.page.getByText(fileName, { exact: false }).first()).toBeVisible({ timeout: 25_000 });

      // B 点击附件触发下载，文件名正确
      const dlPromise = b.page.waitForEvent('download', { timeout: 20_000 });
      await b.page.getByText(fileName, { exact: false }).first().click();
      const dl = await dlPromise;
      expect(dl.suggestedFilename()).toBe(fileName);

      // 图片附件：A 发送 1x1 PNG，B 侧渲染 img 且能解密加载
      const imgName = `pic-${t}.png`;
      const imgMsg = `img-msg-${t}`;
      const imgChooserPromise = a.page.waitForEvent('filechooser');
      await a.page.getByTestId('attachment-picker').click();
      const imgChooser = await imgChooserPromise;
      await imgChooser.setFiles([{ name: imgName, mimeType: 'image/png', buffer: TINY_PNG }]);
      await a.page.getByTestId('message-input').click();
      await a.page.keyboard.type(imgMsg, { delay: 20 });
      await a.page.keyboard.press('Enter');

      const row = messageRowByText(b.page, imgMsg);
      await expect(row).toBeVisible({ timeout: 25_000 });
      // 只匹配附件图片（消息行里还有头像 img）
      const img = row.locator(`img[alt="${imgName}"]`);
      await expect(img).toBeVisible({ timeout: 20_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });

  test('02 search palette finds message and jumps back to channel', async ({ browser }) => {
    test.setTimeout(180_000);
    const users = await spawnUsers(browser, 2, 'sr');
    const cols = users.map((u, i) => attachErrorCollector(u.page, `u${i}`));
    try {
      const [a, b] = users;
      await setupIsolatedChannel(users, 'iso');
      const t = tag();
      const keyword = `SEARCH_${t}`;
      await sendMessage(a.page, `${keyword}_needle`);
      await expect(b.page.getByText(keyword, { exact: false }).first()).toBeVisible({ timeout: 20_000 });

      // 先把 B 切到别的视图，验证搜索结果能跳回频道
      await b.page.getByTestId('nav-more').click();
      await b.page.keyboard.press('Control+K');
      await expect(b.page.getByTestId('search-palette')).toBeVisible();
      await b.page.getByTestId('search-input').click();
      await b.page.keyboard.type(keyword, { delay: 25 });
      const result = b.page.getByTestId('search-result-0');
      await expect(result).toBeVisible({ timeout: 10_000 });
      await expect(result).toContainText(keyword);
      await b.page.keyboard.press('Enter');
      await expect(b.page.getByTestId('search-palette')).toBeHidden({ timeout: 10_000 });
      await expect(b.page.getByTestId('message-input')).toBeVisible({ timeout: 15_000 });
      await expect(b.page.getByText(`${keyword}_needle`, { exact: false }).first()).toBeVisible({ timeout: 15_000 });

      // 无结果状态
      await b.page.keyboard.press('Control+K');
      await b.page.getByTestId('search-input').click();
      await b.page.keyboard.type(`nothing-${t}`, { delay: 20 });
      await expect(b.page.getByTestId('search-empty')).toBeVisible({ timeout: 10_000 });
      await b.page.keyboard.press('Escape');
      await expect(b.page.getByTestId('search-palette')).toBeHidden({ timeout: 10_000 });

      const errs = cols.flatMap(c => c.getFiltered());
      expect(errs, `unexpected errors: ${errs.slice(0, 5).join(' | ')}`).toEqual([]);
    } finally {
      await closeUsers(users);
    }
  });
});

