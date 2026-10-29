// test-msg-copy: 验证消息右键菜单 + 移动端长按菜单
const { checkPrereqs, login, randName, typeMessage } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 test-msg-copy: 消息右键/长按复制菜单\n');
  await checkPrereqs();
  const browser = await launch();
  const pageErrors = [];

  try {
    // ===== 桌面端右键 =====
    console.log('1. 桌面端右键');
    const desktopCtx = await browser.newContext();
    const page = await desktopCtx.newPage();
    page.on('pageerror', e => pageErrors.push(String(e)));

    const name = randName('msgcopy-d');
    await login(page, name);
    console.log('   登录:', name);

    const text = 'msg-copy-test-' + Date.now();
    await typeMessage(page, text);
    console.log('   已发送:', text);

    await page.waitForSelector(`text=${text}`, { timeout: 5000 });
    const msgRow = page.locator('.group.flex.gap-3').filter({ hasText: text }).first();

    await msgRow.click({ button: 'right' });
    await page.waitForSelector('button:has-text("复制文本")', { timeout: 2000 });
    console.log('   ✅ 右键菜单出现');

    await page.click('button:has-text("复制文本")');
    await page.waitForSelector('button:has-text("复制文本")', { state: 'detached', timeout: 2000 });
    console.log('   ✅ 点击复制后关闭');

    await msgRow.click({ button: 'right' });
    await page.waitForSelector('button:has-text("复制文本")', { timeout: 2000 });
    await page.locator('body').click({ position: { x: 10, y: 10 }, force: true });
    await page.waitForSelector('button:has-text("复制文本")', { state: 'detached', timeout: 2000 });
    console.log('   ✅ 点空白处关闭');

    await desktopCtx.close();

    // ===== 移动端长按 =====
    console.log('\n2. 移动端长按');
    const mobileCtx = await browser.newContext({
      viewport: { width: 390, height: 844 },
      hasTouch: true,
      isMobile: true,
    });
    const mpage = await mobileCtx.newPage();
    mpage.on('pageerror', e => pageErrors.push(String(e)));

    const mname = randName('msgcopy-m');
    await login(mpage, mname);
    console.log('   登录:', mname);

    const mtext = 'msg-copy-mobile-' + Date.now();
    await typeMessage(mpage, mtext);
    console.log('   已发送:', mtext);

    await mpage.waitForSelector(`text=${mtext}`, { timeout: 5000 });
    const mRow = mpage.locator('.group.flex.gap-3').filter({ hasText: mtext }).first();

    // 模拟长按：pointerdown → 700ms → pointerup
    await mRow.dispatchEvent('pointerdown', { pointerType: 'touch', clientX: 200, clientY: 400 });
    await mpage.waitForTimeout(700);
    await mRow.dispatchEvent('pointerup', { pointerType: 'touch' });
    await mpage.waitForSelector('button:has-text("复制文本")', { timeout: 2000 });
    console.log('   ✅ 长按菜单出现');

    await mobileCtx.close();

    if (pageErrors.length) {
      console.warn('\n   ⚠️ 捕获到页面错误:', pageErrors.slice(0, 3));
    }

    console.log('\n=== ✅ TEST PASSED ===');
    process.exit(0);
  } catch (e) {
    console.error('\n=== ❌ TEST FAILED ===');
    console.error(e);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
