import {
  test, expect,
  getFirstVoiceChannel, enterVoiceChannel,
  localShareInfo, installGdm4kReject, installGdmNotAllowed, login,
} from '../lib/fixtures';

test.describe('SCREEN SHARE — 降级/取消（精准失败注入）', () => {
  test('请求 4K 被浏览器拒 -> 自动降级 1080p，共享真实可用', async ({ browser }) => {
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true, permissions: ['camera', 'microphone'] });
    const page = await ctx.newPage();
    await installGdm4kReject(page); // 只对 width>1920 抛错，其他放过
    const name = 'u0-4k-' + Math.floor(Math.random() * 1e6);
    await login(page, name);

    const { id: chId, name: vn } = await getFirstVoiceChannel(page);
    await enterVoiceChannel(page, chId, vn);

    // 先切到 4K
    await page.getByTestId('media-quality').selectOption('4k60');
    await page.getByTestId('media-share-toggle').click();

    // 关键：不仅 UI 要显示 on，实际要有发布、panel 要出、settings 在 1080p 上限内
    await expect(page.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'on', { timeout: 30_000 });
    await expect(page.getByTestId('screenshare-panel')).toBeVisible({ timeout: 30_000 });
    await expect(page.getByTestId('screen-share-video').first()).toBeVisible({ timeout: 15_000 });

    const local = await localShareInfo(page);
    expect(local?.has, JSON.stringify(local)).toBe(true);
    expect(local?.settings?.height ?? 0).toBeGreaterThan(0);
    expect(local?.settings?.height ?? 0).toBeLessThanOrEqual(1080);

    await ctx.close();
  });

  test('用户取消共享（NotAllowedError）-> UI 回退，无幽灵 track，无 panel', async ({ browser }) => {
    const ctx = await browser.newContext({ ignoreHTTPSErrors: true, permissions: ['camera', 'microphone'] });
    const page = await ctx.newPage();
    await installGdmNotAllowed(page);
    const name = 'u0-cancel-' + Math.floor(Math.random() * 1e6);
    await login(page, name);

    const { id: chId, name: vn } = await getFirstVoiceChannel(page);
    await enterVoiceChannel(page, chId, vn);

    await page.getByTestId('media-share-toggle').click();
    await expect(page.getByTestId('media-share-toggle')).toHaveAttribute('data-sharing', 'off', { timeout: 15_000 });
    await expect(page.getByTestId('screenshare-panel')).toHaveCount(0);

    const local = await localShareInfo(page);
    expect(local?.has, JSON.stringify(local)).toBe(false);

    await ctx.close();
  });
});
