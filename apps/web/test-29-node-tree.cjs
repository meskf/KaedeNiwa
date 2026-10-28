// test-29: Node Tree (无限目录树) 验证
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }

(async () => {
  console.log('🧪 测试 29: 无限目录树 UI\n');
  await checkPrereqs();
  const browser = await launch();
  const uName = randName('tree');
  try {
    const page = await (await browser.newContext()).newPage();
    const logs = [];
    page.on('console', m => logs.push(m.text()));
    await login(page, uName);
    await page.waitForSelector('[data-testid="message-input"]', { timeout: 15000 });
    console.log('  user =', uName);

    // 等待自动 Seed 完成
    await page.waitForTimeout(3000);

    // Step 1: 断言左侧出现目录树
    const folderCount = await page.locator('text=文字频道').count();
    const generalCount = await page.locator('text=general').count();
    assert(folderCount >= 1, '未自动生成「文字频道」文件夹');
    assert(generalCount >= 1, '未自动生成「general」文本频道');
    console.log('  1. ✅ 自动 Seed 目录树成功');

    // Step 2: 右键点击空白处或文件夹，创建新目录
    // 直接右键「文字频道」文件夹
    const rootFolder = await page.evaluate(() => window.__kaede_debug__.nodes.list().then(nodes => nodes.find(n => n.name === '文字频道')));
    await page.locator(`[data-node-id="${rootFolder.id}"]`).click({ button: 'right' });
    await page.waitForTimeout(500);
    
    // 此时应该弹出 NodeContextMenu
    const menuVisible = await page.locator('[data-testid="ctx-create-folder"]').count();
    assert(menuVisible >= 1, '右键菜单未弹出');
    console.log('  2. ✅ 右键菜单正常');

    // 在菜单里点「新建子文件夹」
    await page.locator('[data-testid="ctx-create-folder"]').click();
    await page.waitForTimeout(500);
    
    // 输入名称并确认
    const testFolderName = 'TestFolder_' + Date.now();
    await page.locator('input[placeholder="输入名称..."]').fill(testFolderName);
    await page.locator('[data-testid="ctx-confirm"]').click();
    await page.waitForTimeout(2000);

    // 校验新文件夹出现
    const newFolderVisible = await page.locator(`text=${testFolderName}`).count();
    if (newFolderVisible === 0) {
      logs.filter(l => /error|fail|not logged/i.test(l)).slice(-10).forEach(l => console.log('    [log]', l));
      throw new Error('新建文件夹后 UI 未刷新');
    }
    console.log('  3. ✅ 新建文件夹成功:', testFolderName);

    // Step 3: 验证 debug 接口里确实有这个节点
    const nodes = await page.evaluate(() => window.__kaede_debug__.nodes.list());
    const found = nodes.find(n => n.name === testFolderName);
    assert(found, '后端无该节点记录');
    assert(found.kind === 'folder', '节点 kind 不是 folder');
    console.log('  4. ✅ 后端已落库');

    // Step 4: 重命名测试
    // 右键新文件夹
    await page.locator(`[data-node-id="${found.id}"]`).click({ button: 'right' });
    await page.waitForTimeout(500);
    await page.locator('[data-testid="ctx-rename"]').click();
    await page.waitForTimeout(500);
    await page.locator('input[placeholder="输入名称..."]').fill(testFolderName + '_renamed');
    await page.locator('[data-testid="ctx-confirm"]').click();
    await page.waitForTimeout(2000);

    const renamedNodes = await page.evaluate(() => window.__kaede_debug__.nodes.list());
    const renamedNode = renamedNodes.find(n => n.name === testFolderName + '_renamed');
    assert(renamedNode, '重命名后后端未更新');
    
    const renamedVisible = await page.locator(`text=${testFolderName}_renamed`).count();
    assert(renamedVisible >= 1, '重命名后 UI 未更新');
    console.log('  5. ✅ 重命名成功');

    // Step 5: 删除测试
    await page.locator(`[data-node-id="${renamedNode.id}"]`).click({ button: 'right' });
    await page.waitForTimeout(500);
    await page.locator('[data-testid="ctx-delete"]').click();
    await page.waitForTimeout(500);
    
    // 注意：这里的 dialog 监听在 Playwright 里需要在操作前设置，为了简单我们直接 evaluate 触发
    await page.evaluate(() => window.confirm = () => true);
    await page.locator('[data-testid="ctx-delete"]').last().click();
    await page.waitForTimeout(2000);

    const deletedVisible = await page.locator(`text=${testFolderName}_renamed`).count();
    assert(deletedVisible === 0, '删除后节点仍然可见');
    console.log('  6. ✅ 删除成功');

    console.log('\n🎉 测试 29 通过');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 29 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
