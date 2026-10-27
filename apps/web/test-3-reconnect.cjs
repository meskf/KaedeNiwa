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
  console.log('🧪 测试 3: 断线重连');
  const browser = await launch();
  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');
    
    const alice = await (await browser.newContext()).newPage();
    await login(alice, aliceName);
    const bob = await (await browser.newContext()).newPage();
    await login(bob, bobName);
    
    await alice.waitForTimeout(4000);
    
    await typeMessage(alice, 'BEFORE_DISCONNECT');
    await alice.waitForTimeout(1500);
    
    await bob.reload();
    await bob.waitForSelector('[data-testid="message-input"]', { timeout: 15000 });
    await bob.waitForTimeout(3000);
    
    const bobHasMsg = await bob.locator('text=BEFORE_DISCONNECT').isVisible();
    
    if (bobHasMsg) {
      console.log('🎉 测试 3 通过：重连后能看到历史消息');
    } else {
      console.log('❌ 测试 3 失败：重连后看不到历史消息');
    }
  } finally {
    await browser.close();
  }
})();
