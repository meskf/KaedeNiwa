// test-21: Settings 头像/资料 + 服务器图标 UI 验证
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 test-21: 头像/资料/服务器图标 UI\n');
  await checkPrereqs();
  const browser = await launch({ args: ['--use-fake-ui-for-media-stream'] });
  const pageLogs = [];
  try {
    const page = await (await browser.newContext()).newPage();
    page.on('console', m => pageLogs.push(m.text()));

    const name = randName('ui21');
    await login(page, name);
    console.log('👤 login:', name);

    // 等主页渲染完再点 settings
    await page.waitForSelector('[data-testid="nav-settings"]', { timeout: 15000 });
    console.log('1. 进入 Settings...');
    await page.click('[data-testid="nav-settings"]');

    // 等账户 section 出现
    await page.waitForSelector('[data-testid="settings-nickname-input"]', { timeout: 10000 });
    console.log('   ✅ Settings 已进入');

    // 2. 检查账户 section 元素
    console.log('2. 检查账户 section 元素...');
    const ids = [
      'settings-avatar-button',
      'settings-nickname-input',
      'settings-bio-input',
      'settings-title-input',
      'settings-profile-save',
      'settings-avatar-file',
      'settings-server-icon-button',
      'settings-server-icon-file',
    ];
    for (const id of ids) {
      const c = await page.locator(`[data-testid="${id}"]`).count();
      console.log(`   ${id} = ${c}`);
      if (c === 0) throw new Error(`元素缺失: ${id}`);
    }

    // 3. 上传头像
    console.log('3. 上传头像...');
    const jpegDataUrl = await page.evaluate(() => {
      const c = document.createElement('canvas');
      c.width = 256; c.height = 256;
      const ctx = c.getContext('2d');
      ctx.fillStyle = '#3498db'; ctx.fillRect(0, 0, 256, 256);
      ctx.fillStyle = '#fff'; ctx.font = 'bold 100px sans-serif'; ctx.textAlign = 'center';
      ctx.fillText('A', 128, 170);
      return c.toDataURL('image/jpeg', 0.85);
    });
    const jpegBuf = Buffer.from(jpegDataUrl.replace(/^data:image\/jpeg;base64,/, ''), 'base64');
    console.log('   jpeg buffer size =', jpegBuf.length);

    await page.setInputFiles('[data-testid="settings-avatar-file"]', {
      name: 'avatar.jpg', mimeType: 'image/jpeg', buffer: jpegBuf,
    });
    await page.waitForTimeout(2500);
    const avatarImg = await page.locator('[data-testid="settings-avatar-button"] img').count();
    console.log('   头像 <img> 出现 =', avatarImg);
    if (!avatarImg) {
      pageLogs.filter(l => /avatar|upload|error|fetch/i.test(l)).slice(-8).forEach(l => console.log('    [log]', l));
      throw new Error('上传后头像未显示');
    }

    // 4. 改昵称/简介/称号并保存
    console.log('4. 改昵称/简介/称号 + 保存...');
    await page.fill('[data-testid="settings-nickname-input"]', 'Nick21');
    await page.fill('[data-testid="settings-bio-input"]', 'Bio from test-21');
    await page.fill('[data-testid="settings-title-input"]', 'UI Tester');
    await page.click('[data-testid="settings-profile-save"]');
    await page.waitForTimeout(1500);

    // 5. 上传服务器图标
    console.log('5. 上传服务器图标...');
    await page.setInputFiles('[data-testid="settings-server-icon-file"]', {
      name: 'icon.jpg', mimeType: 'image/jpeg', buffer: jpegBuf,
    });
    await page.waitForTimeout(2500);
    const iconImg = await page.locator('[data-testid="settings-server-icon-button"] img').count();
    console.log('   服务器图标 <img> 出现 =', iconImg);
    if (!iconImg) {
      pageLogs.filter(l => /icon|upload|error|fetch|forbidden/i.test(l)).slice(-8).forEach(l => console.log('    [log]', l));
      throw new Error('服务器图标上传后未显示');
    }

    // 6. StageManagerServers 显示
    console.log('6. 检查 StageManagerServers 图标引用...');
    await page.waitForTimeout(1000);
    const stageImgs = await page.locator('img[src*="/api/server/icon"]').count();
    console.log('   页面 /api/server/icon 引用数 =', stageImgs);
    if (stageImgs < 1) throw new Error('StageManagerServers 没有渲染图标');

    console.log('\n🎉 test-21 通过');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('❌ test-21 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
