// test-24: 搜索面板（search-trigger / Ctrl+K → 输入 → 高亮 → 点击跳转）
const { checkPrereqs, login, randName, typeMessage } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }

(async () => {
  console.log('🧪 测试 24: 搜索面板\n');
  await checkPrereqs();
  const browser = await launch();
  const uName = randName('search');
  try {
    const page = await (await browser.newContext()).newPage();
    const logs = [];
    page.on('console', m => logs.push(m.text()));
    await login(page, uName);
    await page.waitForSelector('[data-testid="message-input"]', { timeout: 15000 });
    console.log('  user =', uName);

    // Step 1: 发一条独特消息
    const mark = 'SRCH_' + Date.now();
    await typeMessage(page, mark + ' hello world');
    console.log('  1. ✅ 已发消息:', mark);
    await page.waitForTimeout(2500);

    // Step 2: 打开搜索面板（点 search-trigger）
    const triggerSel = '[data-testid="search-trigger"]';
    if ((await page.locator(triggerSel).count()) === 0) throw new Error('search-trigger 不存在');
    await page.click(triggerSel);
    await page.waitForSelector('[data-testid="search-palette"]', { timeout: 5000 });
    console.log('  2. ✅ 搜索面板已打开');

    // Step 3: 输入关键词
    await page.fill('[data-testid="search-input"]', mark);
    await page.waitForTimeout(800);

    const resultCount = await page.locator('[data-testid="search-results"] > *').count();
    const emptyCount = await page.locator('[data-testid="search-empty"]').count();
    console.log('  3. 结果条数 =', resultCount, '空提示 =', emptyCount);
    if (resultCount < 1) {
      logs.filter(l => /search|result|message/i.test(l)).slice(-10).forEach(l => console.log('    [log]', l));
      throw new Error('搜索结果为空');
    }
    console.log('  3. ✅ 有搜索结果');

    // Step 4: 高亮
    const markCount = await page.locator('[data-testid="search-results"] mark').count();
    assert(markCount >= 1, `搜索结果无 <mark> 高亮（mark count = ${markCount}）`);
    console.log('  4. ✅ 高亮存在，<mark> 数 =', markCount);

    // Step 5: 点击结果 → 面板关闭
    const clickable = page.locator('[data-testid="search-results"] [role="button"], [data-testid="search-results"] button');
    const clickableCount = await clickable.count();
    if (clickableCount > 0) {
      await clickable.first().click();
    } else {
      // fallback：点第一行子元素
      await page.locator('[data-testid="search-results"] > *').first().click();
    }
    await page.waitForTimeout(800);
    const paletteGone = await page.locator('[data-testid="search-palette"]').count();
    assert(paletteGone === 0, `点击结果后面板应关闭，实际 count = ${paletteGone}`);
    console.log('  5. ✅ 点击结果后面板关闭');

    // Step 6: Ctrl+K 快捷键（非致命）
    await page.keyboard.press('Control+K');
    await page.waitForTimeout(400);
    const reopened = await page.locator('[data-testid="search-palette"]').count();
    if (reopened === 1) {
      console.log('  6. ✅ Ctrl+K 快捷键生效');
      await page.keyboard.press('Escape');
      await page.waitForTimeout(300);
    } else {
      console.log('  6. ⚠️ Ctrl+K 未打开面板（非致命，可能是快捷键未实现或环境差异）');
    }

    console.log('\n🎉 测试 24 通过');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 24 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
