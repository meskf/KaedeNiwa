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
    
    // 绕过 UI，直接在 voice 频道发送消息测试隔离
    await alice.evaluate(async () => {
      const wasm = await import('/src/wasm/client.ts');
      const c = wasm.getClient();
      if (c) await c.send_message('voice', 'SECRET_DEV_MSG');
    });
    await alice.waitForTimeout(1500);
    await typeMessage(alice, 'SECRET_DEV_MSG');
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
