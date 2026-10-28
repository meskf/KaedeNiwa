const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newContext().then(c => c.newPage());
  
  console.log('1. 注册一个用户（此时 DB 里可能有之前遗留的僵尸号）...');
  await page.goto('http://localhost:5173');
  await page.waitForTimeout(800);
  const name = 'solo-' + Math.floor(Math.random() * 1e6);
  await page.locator('[data-testid="login-id"]').fill(name);
  await page.locator('[data-testid="login-password"]').fill('password123');
  await page.locator('[data-testid="login-toggle-register"]').click();
  await page.waitForTimeout(200);
  await page.locator('[data-testid="login-submit"]').click();
  await page.waitForSelector('[data-testid="message-input"]', { timeout: 20000 });
  await page.waitForTimeout(2000);

  console.log('2. 双击进入语音频道（尝试加入僵尸群）...');
  const voiceChannelBtn = page.locator('div:text-is("语音")').first();
  await voiceChannelBtn.click();
  await page.waitForTimeout(1000);
  await voiceChannelBtn.dblclick();
  
  console.log('3. 等待 12 秒观察是否报错（预期：因为全是离线号，无法发 Welcome）...');
  await page.waitForTimeout(12000);

  // 检查是否出现错误
  const bodyText = await page.evaluate(() => document.body.innerText);
  const hasError = bodyText.includes('Join voice failed') || bodyText.includes('welcome channel closed');
  
  console.log('\n=== SOLO-ZOMBIE REPORT ===');
  console.log('Has Error:', hasError);
  console.log('PASS:', !hasError);
  await browser.close();
  process.exit(!hasError ? 0 : 1);
})();
