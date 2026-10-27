const { chromium } = require('playwright');

async function login(page, name) {
  await page.goto('http://localhost:5173');
  await page.waitForSelector('[data-testid="login-id"]', { timeout: 15000 });
  await page.fill('[data-testid="login-id"]', name);
  await page.fill('[data-testid="login-password"]', 'password123');
  const reg = page.locator('[data-testid="login-toggle-register"]');
  if (await reg.isVisible()) await reg.click();
  await page.click('[data-testid="login-submit"]');
  await page.waitForSelector('[data-testid="message-input"]', { timeout: 15000 });
}

function randName(prefix) {
  return prefix + '-' + Math.floor(Math.random() * 100000);
}


(async () => {
  console.log('🧪 测试 1: 自动拉人');
  const browser = await chromium.launch({ headless: false, channel: 'msedge' });
  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');
    
    const alice = await (await browser.newContext()).newPage();
    await login(alice, aliceName);
    console.log('✅ Alice 登录:', aliceName);

    const bob = await (await browser.newContext()).newPage();
    await login(bob, bobName);
    console.log('✅ Bob 登录:', bobName);

    await alice.waitForTimeout(4000);
    const status = await alice.locator('[data-testid="peer-status"]').innerText().catch(() => '');
    console.log('📊 拉人状态:', status);

    const bobEvents = await bob.locator('[data-testid="raw-event"]').allInnerTexts();
    const joined = bobEvents.find(e => e.includes('ChannelJoined'));
    if (joined) {
      console.log('🎉 测试 1 通过：Bob 收到 ChannelJoined');
    } else {
      console.log('❌ 测试 1 失败：Bob 未收到 ChannelJoined');
    }
  } finally {
    await browser.close();
  }
})();
