// test-23: 分类 + 频道 CRUD（wasm 直调）
// 覆盖：初始系统分类 → 创建分类 → 创建频道 → 重命名 → 删除频道 → 删除分类
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }
function pickId(obj) {
  if (!obj) return null;
  if (typeof obj === 'string') return obj;
  return obj.id || obj.category_id || obj.channel_id || null;
}

(async () => {
  console.log('🧪 测试 23: 分类 + 频道 CRUD\n');
  await checkPrereqs();
  const browser = await launch();
  const uName = randName('crud');
  let newCatId = null;
  let newChId = null;
  try {
    const page = await (await browser.newContext()).newPage();
    const logs = [];
    page.on('console', m => logs.push(m.text()));
    await login(page, uName);
    await page.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.channels && window.__kaede_debug__.categories), null, { timeout: 10000 });
    console.log('  user =', uName);

    // Step 1: 初始分类
    const cats0 = await page.evaluate(() => window.__kaede_debug__.categories.list());
    assert(Array.isArray(cats0), 'categories.list 应返回数组');
    console.log('  1. ✅ 初始分类:', cats0.map(c => c.id).join(',') || '(空)');

    // Step 2: 创建分类
    const catName = 'TEST_CAT_' + Date.now();
    const created = await page.evaluate(async (name) => window.__kaede_debug__.categories.create(name), catName);
    newCatId = pickId(created);
    assert(newCatId, `createCategory 未返回可识别 id，原始返回: ${JSON.stringify(created).slice(0, 200)}`);
    console.log('  2. ✅ 创建分类 id =', newCatId);

    // Step 3: list 应包含
    const cats1 = await page.evaluate(() => window.__kaede_debug__.categories.list());
    assert(cats1.some(c => c.id === newCatId), `新分类未出现在 list，实际 ids: ${cats1.map(c => c.id).join(',')}`);
    console.log('  3. ✅ 分类已列出');

    // Step 4: 在分类下建频道
    const chName = 'test-ch-' + Date.now();
    const createdCh = await page.evaluate(async ({ name, catId }) => window.__kaede_debug__.channels.create(name, 'text', catId, false, null), { name: chName, catId: newCatId });
    newChId = pickId(createdCh);
    assert(newChId, `createChannel 未返回可识别 id，原始返回: ${JSON.stringify(createdCh).slice(0, 200)}`);
    console.log('  4. ✅ 创建频道 id =', newChId);

    // Step 5: listChannels 应包含，且 category_id 正确
    const chs1 = await page.evaluate(() => window.__kaede_debug__.channels.list());
    const f1 = chs1.find(c => c.id === newChId);
    assert(f1, `新频道未出现在 listChannels，实际 ids: ${chs1.map(c => c.id).join(',')}`);
    assert(f1.name === chName, `频道名应为 ${chName}，实际 ${f1.name}`);
    assert(f1.category_id === newCatId, `频道 category_id 应为 ${newCatId}，实际 ${f1.category_id}`);
    console.log('  5. ✅ 频道在正确分类下，名称正确');

    // Step 6: 重命名
    const renamed = chName + '_renamed';
    await page.evaluate(async ({ id, name }) => window.__kaede_debug__.channels.update(id, name, null), { id: newChId, name: renamed });
    const chs2 = await page.evaluate(() => window.__kaede_debug__.channels.list());
    const f2 = chs2.find(c => c.id === newChId);
    assert(f2 && f2.name === renamed, `重命名后名应为 ${renamed}，实际 ${f2 && f2.name}`);
    console.log('  6. ✅ 频道重命名生效');

    // Step 7: 删除频道
    const toDelCh = newChId;
    await page.evaluate((id) => window.__kaede_debug__.channels.delete(id), toDelCh);
    newChId = null;
    await page.waitForTimeout(500);
    const chs3 = await page.evaluate(() => window.__kaede_debug__.channels.list());
    assert(!chs3.some(c => c.id === toDelCh), '删除后频道仍在');
    console.log('  7. ✅ 频道已删除');

    // Step 8: 删除分类
    const toDelCat = newCatId;
    await page.evaluate((id) => window.__kaede_debug__.categories.delete(id), toDelCat);
    newCatId = null;
    await page.waitForTimeout(500);
    const cats2 = await page.evaluate(() => window.__kaede_debug__.categories.list());
    assert(!cats2.some(c => c.id === toDelCat), '删除后分类仍在');
    console.log('  8. ✅ 分类已删除');

    console.log('\n🎉 测试 23 通过');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 23 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
