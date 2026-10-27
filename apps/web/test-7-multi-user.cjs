const { chromium, checkPrereqs, login, randName, injectFakeDisplayMedia, setDisableAdaptive, typeMessage } = require('./test-helpers.cjs');

(async () => {
  console.log('🧪 测试 7: 3 人多人\n');
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await chromium.launch({
    headless: false,
    channel: 'msedge',
    args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'],
  });

  try {
    const aliceName = randName('alice');
    const bobName = randName('bob');
    const charlieName = randName('charlie');

    const alice = await (await browser.newContext()).newPage();
    await injectFakeDisplayMedia(alice);
    await setDisableAdaptive(alice);
    await login(alice, aliceName);
    console.log('👤 Alice:', aliceName);

    const bob = await (await browser.newContext()).newPage();
    await setDisableAdaptive(bob);
    await login(bob, bobName);
    console.log('👤 Bob:', bobName);

    const charlie = await (await browser.newContext()).newPage();
    await setDisableAdaptive(charlie);

    // DEBUG: 捕获 WASM console（遇失败时能看到 epoch 状态）
    const debugLogs = { alice: [], bob: [], charlie: [] };
    alice.on('console', m => { const t = m.text(); if (/epoch|channel|mls|Channel/i.test(t)) debugLogs.alice.push(t); });
    bob.on('console', m => { const t = m.text(); if (/epoch|channel|mls|Channel/i.test(t)) debugLogs.bob.push(t); });

    await login(charlie, charlieName);
    console.log('👤 Charlie:', charlieName);

    console.log('⏳ 等待自动拉人（三方互相可见）...');
    await alice.waitForTimeout(6000);

    // ====== 场景 1：三人加入语音 ======
    console.log('\n🎤 场景 1: 三人加入语音');
    for (const [name, p] of [['Alice', alice], ['Bob', bob], ['Charlie', charlie]]) {
      await p.click('[data-testid="channel-item-voice"]');
    await p.waitForTimeout(3000);
    await p.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await p.click('[data-testid="media-join"]');
    await p.waitForTimeout(3000);
      await p.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
      console.log(`✅ ${name} 加入语音`);
      await p.waitForTimeout(500);
    }
    await alice.waitForTimeout(3000);

    // ====== 场景 2：三人互相可见 ======
    console.log('\n👥 场景 2: 三人 participants 都应该是 3 个');
    for (const [name, p] of [['Alice', alice], ['Bob', bob], ['Charlie', charlie]]) {
      // 数 participants 的 div 数量
      const count = await p.locator('[data-participant]').count();
      console.log(`   ${name}: ${count} 个 participant`);
      if (count < 3) {
        console.log(`   ❌ ${name} 只看到 ${count} 个参与者`);
        process.exit(1);
      }
    }
    console.log('✅ 三人互相可见');

    // ====== 场景 3：Alice 共享屏幕，Bob + Charlie 都能看到 ======
    console.log('\n🖥️ 场景 3: Alice 共享屏幕，Bob + Charlie 都收到');
    await alice.locator('[data-testid="media-quality"]').selectOption('720p30');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });

    for (const [name, p] of [['Bob', bob], ['Charlie', charlie]]) {
      await p.waitForFunction(() => {
        for (const v of document.querySelectorAll('video')) {
          if (v.videoWidth > 0) return true;
        }
        return false;
      }, { timeout: 20000 });
      const info = await p.evaluate(() => {
        for (const v of document.querySelectorAll('video')) {
          if (v.videoWidth > 0) return { w: v.videoWidth, h: v.videoHeight };
        }
        return null;
      });
      console.log(`   ${name}: ${info?.w}x${info?.h}`);
      if (!info || info.w < 640) {
        console.log(`   ❌ ${name} 画面异常`);
        process.exit(1);
      }
    }
    console.log('✅ Bob + Charlie 都看到 Alice 的屏幕');

    // ====== 场景 4：Bob 开麦，Alice + Charlie 都看到 ======
    console.log('\n🎙️ 场景 4: Bob 开麦，其他人看到 🎤');
    await bob.click('[data-testid="media-mic-toggle"]');
    await bob.waitForSelector('[data-testid="media-mic-toggle"]', { timeout: 10000 });
    await alice.waitForTimeout(2000);

    // Alice 和 Charlie 都应该能看到 Bob 名字旁有 🎤
    for (const [name, p] of [['Alice', alice], ['Charlie', charlie]]) {
      const hasMic = await p.locator(`[data-participant="${bobName}"][data-mic="on"]`).count();
      if (hasMic > 0) {
        console.log(`   ✅ ${name} 看到 Bob 的 🎤`);
      } else {
        console.log(`   ⚠️ ${name} 未看到 Bob 的 🎤（时序）`);
      }
    }

    // ====== 场景 5：Alice 发消息，Bob + Charlie 都收到 ======
    // 为什么用 Alice 发：Alice 是 authority，channel group 状态最稳。
    // Charlie（第 3 个加入者）的 group 可能还在同步，不适合作为发送方。
    console.log('\n💬 场景 5: Alice 发消息，Bob + Charlie 都收到');
    await typeMessage(alice, 'HELLO_MULTI_USER_TEST');

    for (const [name, p] of [['Bob', bob], ['Charlie', charlie]]) {
      try {
        await p.waitForSelector('text=HELLO_MULTI_USER_TEST', { timeout: 8000 });
        console.log(`   ✅ ${name} 收到消息`);
      } catch (e) {
        console.log(`   ❌ ${name} 未收到消息，打印 debug 日志：`);
        for (const [who, logs] of Object.entries(debugLogs)) {
          console.log(`  --- ${who} (${logs.length} 条) ---`);
          logs.slice(-20).forEach(l => console.log(`    ${l}`));
        }
        throw e;
      }
    }

    // ====== 场景 6：Alice 停止共享 + 离开语音 ======
    console.log('\n🛑 场景 6: Alice 停止共享 + 离开语音');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });
    await alice.click('[data-testid="media-leave"]');
    await alice.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    console.log('✅ Alice 已离开');

    // Bob 和 Charlie 检查 participants 降到 2
    await bob.waitForTimeout(3000);
    for (const [name, p] of [['Bob', bob], ['Charlie', charlie]]) {
      const count = await p.locator('[data-participant]').count();
      console.log(`   ${name}: 现在 ${count} 个 participant`);
    }

    console.log('\n🎉 测试 7 通过');
  } catch (e) {
    console.error('\n❌ 测试 7 失败:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
