import { test, expect, waitDebug } from './fixtures';

test.describe('CRUD & SEARCH', () => {
  test('频道完整生命周期：创建 -> 重命名 -> 删除', async ({ u0 }) => {
    await waitDebug(u0);
    await u0.waitForTimeout(2000);

    const chName = `test-ch-${Date.now()}`;
    
    // 1. 创建频道
    const created = await u0.evaluate((name) => {
      const dbg = (window as any).__kaede_debug__;
      return dbg.channels.create(name, 'text', null, false, null);
    }, chName);
    expect(created.id).toBeTruthy();

    // 2. 验证列表
    let channels = await u0.evaluate(() => (window as any).__kaede_debug__.channels.list());
    expect(channels.some((c: any) => c.name === chName)).toBe(true);

    // 3. 重命名
    const newName = `${chName}-renamed`;
    await u0.evaluate(({ id, name }) => {
      const dbg = (window as any).__kaede_debug__;
      return dbg.channels.update(id, name, null);
    }, { id: created.id, name: newName });
    await u0.waitForTimeout(500);

    channels = await u0.evaluate(() => (window as any).__kaede_debug__.channels.list());
    expect(channels.some((c: any) => c.name === newName)).toBe(true);

    // 4. 删除
    await u0.evaluate((id) => {
      const dbg = (window as any).__kaede_debug__;
      return dbg.channels.delete(id);
    }, created.id);
    await u0.waitForTimeout(500);

    channels = await u0.evaluate(() => (window as any).__kaede_debug__.channels.list());
    expect(channels.some((c: any) => c.id === created.id)).toBe(false);
  });

  test('全局搜索：按关键字搜出历史消息', async ({ u0 }) => {
    await waitDebug(u0);
    await u0.waitForTimeout(2000);

    const keyword = `SEARCH_ME_${Date.now()}`;
    const chId = await u0.evaluate(async () => {
      const chList = await (window as any).__kaede_debug__.channels.list();
      return chList.find((c: any) => c.kind === 'text').id;
    });
    
    await u0.evaluate(({ ch, text }) => (window as any).__kaede_debug__.sendMessage(ch, text), { ch: chId, text: keyword });
    await u0.waitForTimeout(1500);

    // 1. 点击只读的 search trigger
    const searchTrigger = u0.locator('[data-testid="search-trigger"]');
    if (await searchTrigger.isVisible()) {
      await searchTrigger.click();
    } else {
      await u0.keyboard.press('Control+k');
    }
    
    // 2. 等待搜索面板弹出，找到真正可编辑的 input（去掉了 readonly 的那个）
    await u0.waitForTimeout(800); // 给面板打开动画一点时间
    const realInput = u0.locator('input[type="text"]:not([readonly])').first();
    await realInput.waitFor({ state: 'visible', timeout: 5000 });
    await realInput.fill(keyword);
    
    // 3. 等待防抖/请求完成，验证结果出现
    await u0.waitForTimeout(2000);
    await expect(u0.getByText(keyword).first()).toBeVisible();
  });
});
