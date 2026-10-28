const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const aCtx = await browser.newContext();
  const bCtx = await browser.newContext();
  const a = await aCtx.newPage();
  const b = await bCtx.newPage();

  async function login(page, name) {
    await page.goto('http://localhost:5173');
    await page.waitForTimeout(1000);
    await page.locator('[data-testid="login-id"]').fill(name);
    await page.locator('[data-testid="login-password"]').fill('password123');
    await page.locator('[data-testid="login-toggle-register"]').click();
    await page.waitForTimeout(200);
    await page.locator('[data-testid="login-submit"]').click();
    await page.waitForSelector('[data-testid="message-input"]', { timeout: 20000 });
    // 等待主群 MLS 就绪（轮询等待）
    for (let i = 0; i < 10; i++) {
      const mainReady = await page.evaluate(async () => {
        try { return (await window.__kaede_debug__.channelState('general')).main_ready; } catch { return false; }
      });
      if (mainReady) break;
      await page.waitForTimeout(1000);
    }
    await page.waitForTimeout(1000);
  }

  console.log('1. 注册并登录 Alice 和 Bob...');
  await login(a, 'alice-v-' + Date.now());
  await login(b, 'bob-v-' + Date.now());

  console.log('2. 获取真实的语音频道 ID...');
  async function getVoiceChannelId(page) {
    return await page.evaluate(() => {
      // 从左侧的 DOM 节点抓取带 data-testid 的频道 ID
      const els = Array.from(document.querySelectorAll('[data-testid^="channel-item-"]'));
      for (const el of els) {
        if (el.textContent?.includes('语音') || el.textContent?.includes('Voice')) {
          return el.getAttribute('data-testid')?.replace('channel-item-', '');
        }
      }
      return null;
    });
  }
  const voiceChId = await getVoiceChannelId(a);
  console.log('   找到语音频道 ID:', voiceChId);

  if (!voiceChId) {
    console.error('FAIL: 找不到语音频道，请检查 UI 是否有「语音」字样');
    process.exit(1);
  }

  console.log('3. 强行同步双击进入语音频道...');
  const voiceSelector = `[data-testid="channel-item-${voiceChId}"]`;

  await a.locator(voiceSelector).click();
  await b.locator(voiceSelector).click();
  await a.waitForTimeout(500);
  await b.waitForTimeout(500);

  await Promise.all([
    a.locator(voiceSelector).dblclick(),
    b.locator(voiceSelector).dblclick()
  ]);

  console.log('4. 等待 UI 出现 Leave Voice...');
  await a.waitForSelector('text=Leave Voice', { timeout: 15000 }).catch(() => console.warn('  Alice 未出现 Leave Voice'));
  await b.waitForSelector('text=Leave Voice', { timeout: 15000 }).catch(() => console.warn('  Bob 未出现 Leave Voice'));

  console.log('5. 等待 4 秒让底层 MLS 同步...');
  await a.waitForTimeout(4000);

  async function getState(page) {
    return await page.evaluate(async (chId) => {
      return await window.__kaede_debug__.channelState(chId);
    }, voiceChId);
  }

  const aState = await getState(a);
  const bState = await getState(b);

  console.log('\n=== ALICE STATE ===');
  console.log(aState);
  console.log('\n=== BOB STATE ===');
  console.log(bState);

  const pass =
    aState && bState &&
    aState.has_group === true &&
    bState.has_group === true &&
    aState.key_b64 === bState.key_b64 &&
    aState.key_b64 !== null;

  console.log('\n=== VOICE MLS REPORT ===');
  console.log('Alice has_group:', aState?.has_group, 'Epoch:', aState?.epoch, 'MainReady:', aState?.main_ready);
  console.log('Bob   has_group:', bState?.has_group, 'Epoch:', bState?.epoch, 'MainReady:', bState?.main_ready);
  console.log('Key Match:', aState?.key_b64 === bState?.key_b64);
  console.log('PASS:', pass);

  await browser.close();
  process.exit(pass ? 0 : 1);
})();
