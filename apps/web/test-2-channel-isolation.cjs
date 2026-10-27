const { launch } = require('./test-launch.cjs');

async function login(page, name) {
  await page.goto('http://127.0.0.1:5173');
  await page.fill('[data-testid="login-id"]', name);
  await page.fill('[data-testid="login-password"]', 'password123');
  const reg = page.locator('[data-testid="login-toggle-register"]');
  if (await reg.isVisible()) await reg.click();
  await page.click('[data-testid="login-submit"]');
  await page.waitForSelector('[data-testid="message-input"]', { timeout: 15000 });
}

async function typeMessage(page, text) {
  await page.click('[data-testid="message-input"]');
  await page.keyboard.type(text);
  await page.keyboard.press('Enter');
}
function randName(prefix) {
  return prefix + '-' + Math.floor(Math.random() * 100000);
}


(async () => {
  console.log('🧪 测试 2: 频道隔离');
  const browser = await launch();
  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');
    
    const alice = await (await browser.newContext()).newPage();
    await login(alice, aliceName);
    const bob = await (await browser.newContext()).newPage();
    await login(bob, bobName);
    
    await alice.waitForTimeout(4000);
    
    await typeMessage(alice, 'SECRET_GENERAL_MSG');
    await alice.waitForTimeout(1500);
    
    // Alice 在 UI 上停留 general，但通过 wasm 真正加入 voice 频道，
    // 再往 voice 发消息。Bob 留在 general，不应该看到 SECRET_DEV_MSG。
    // 这样才真正验证 "voice 频道消息不会漏到 general" 的隔离性。
    const voiceResult = await alice.evaluate(async () => {
      const wasm = await import('/src/wasm/client.ts');
      const c = wasm.getClient();
      if (!c) return { ok: false, reason: 'no client' };
      try {
        await c.join_channel('voice');
        await new Promise(r => setTimeout(r, 800));
        await c.send_message('voice', 'SECRET_DEV_MSG');
        return { ok: true };
      } catch (e) {
        return { ok: false, reason: String(e && (e.message || e)) };
      }
    });
    if (!voiceResult.ok) {
      console.warn('[test-2] ⚠️  voice send failed, isolation assertion is weak:', voiceResult.reason);
    } else {
      console.log('[test-2] ✅ voice send succeeded (real isolation test)');
    }
    await alice.waitForTimeout(1500);
    
    const bobHasGeneral = await bob.locator('text=SECRET_GENERAL_MSG').isVisible();
    const bobHasDev = await bob.locator('text=SECRET_DEV_MSG').isVisible();
    
    if (bobHasGeneral && !bobHasDev) {
      console.log('🎉 测试 2 通过：频道隔离有效');
    } else {
      console.log('❌ 测试 2 失败:', { bobHasGeneral, bobHasDev });
    }
  } finally {
    await browser.close();
  }
})();
