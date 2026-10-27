const { chromium, checkPrereqs, login, randName, injectFakeDisplayMedia, setDisableAdaptive, typeMessage } = require('./test-helpers.cjs');

(async () => {
  console.log('🧪 测试 9: 第一人（authority）退出后的语音连续性\n');
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await chromium.launch({
    headless: false,
    channel: 'msedge',
    args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'],
  });

  try {
    const aliceName = randName('alice');   // 第一个，authority
    const bobName = randName('bob');
    const charlieName = randName('charlie');
    const daveName = randName('dave');    // 后续新人

    // === 阶段 1：三人登录（Alice 是 authority） ===
    console.log('👥 阶段 1: 三人登录');
    const alice = await (await browser.newContext()).newPage();
    await injectFakeDisplayMedia(alice);
    await setDisableAdaptive(alice);
    await login(alice, aliceName);
    console.log(`   Alice (authority): ${aliceName}`);

    const bob = await (await browser.newContext()).newPage();
    await setDisableAdaptive(bob);
    // DEBUG: 挂 Bob/Charlie console
    bob.on('console', m => {
      const t = m.text();
      if (/ChannelMessage|channel_process_commit|send_text|has_group|WrongEpoch|decrypt/i.test(t)) {
        console.log('[Bob]', t);
      }
    });
    await login(bob, bobName);
    console.log(`   Bob: ${bobName}`);

    const charlie = await (await browser.newContext()).newPage();
    await setDisableAdaptive(charlie);
    charlie.on('console', m => {
      const t = m.text();
      if (/ChannelMessage|channel_process_commit|WrongEpoch|decrypt/i.test(t)) {
        console.log('[Charlie]', t);
      }
    });
    await login(charlie, charlieName);
    console.log(`   Charlie: ${charlieName}`);

    console.log('⏳ 等自动拉人...');
    await alice.waitForTimeout(8000);

    // === 阶段 2：三人加入语音 ===
    console.log('\n🎤 阶段 2: 三人加入语音');
    for (const [name, p] of [['Alice', alice], ['Bob', bob], ['Charlie', charlie]]) {
      await p.click('[data-testid="channel-item-voice"]');
    await p.waitForTimeout(3000);
    await p.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await p.click('[data-testid="media-join"]');
    await p.waitForTimeout(3000);
      await p.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
      console.log(`   ✅ ${name}`);
      await p.waitForTimeout(800);
    }
    await alice.waitForTimeout(2000);

    // === 阶段 3：Alice 共享屏幕 ===
    console.log('\n🖥️  阶段 3: Alice 共享屏幕');
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
      console.log(`   ✅ ${name} 收到画面`);
    }

    // === 阶段 4：Alice（authority）离开语音 ===
    console.log('\n🚪 阶段 4: Alice（authority）停止共享 + 离开语音');
    await alice.click('[data-testid="media-share-toggle"]');
    await alice.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });
    await alice.click('[data-testid="media-leave"]');
    await alice.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    console.log('   ✅ Alice 已离开');
    await bob.waitForTimeout(3000);

    // === 诊断：dump Bob + Charlie 的 channel state ===
    console.log('\n🔬 诊断: 对比 Bob vs Charlie 的 channel state');
    const bobState = await bob.evaluate(() => {
      const d = window.__kaede_debug__;
      return d ? d.channelState('general') : null;
    }).catch(e => ({ error: String(e) }));
    const charlieState = await charlie.evaluate(() => {
      const d = window.__kaede_debug__;
      return d ? d.channelState('general') : null;
    }).catch(e => ({ error: String(e) }));
    console.log('   Bob:', JSON.stringify(bobState, null, 2));
    console.log('   Charlie:', JSON.stringify(charlieState, null, 2));
    if (bobState && charlieState && bobState.key_b64 && charlieState.key_b64) {
      console.log('   密钥一致: ' + (bobState.key_b64 === charlieState.key_b64 ? '✅' : '❌'));
      console.log('   epoch 一致: ' + (bobState.epoch === charlieState.epoch ? '✅' : ('❌ (Bob=' + bobState.epoch + ', Charlie=' + charlieState.epoch + ')')));
    }

    // === 阶段 5：Bob + Charlie 语音状态验证 ===
    console.log('\n🔍 阶段 5: 验证 Bob + Charlie 语音功能是否仍然可用');

    // 5.1 双方仍然互相可见
    const bobSeesCharlie = await bob.locator(`text=${charlieName}`).count();
    const charlieSeesBob = await charlie.locator(`text=${bobName}`).count();
    console.log(`   Bob 看到 Charlie: ${bobSeesCharlie > 0 ? '✅' : '❌'}`);
    console.log(`   Charlie 看到 Bob: ${charlieSeesBob > 0 ? '✅' : '❌'}`);

    // 5.2 Bob 开麦，Charlie 能看到
    console.log('   Bob 开麦...');
    await bob.click('[data-testid="media-mic-toggle"]');
    await bob.waitForSelector('[data-testid="media-mic-toggle"]', { timeout: 10000 });
    await bob.waitForTimeout(2000);
    const charlieSeesMic = await charlie.locator(`[data-participant="${bobName}"][data-mic="on"]`).count();
    console.log(`   Charlie 看到 Bob 🎤: ${charlieSeesBob > 0 && charlieSeesMic > 0 ? '✅' : '❌'}`);

    // 5.3 Bob 和 Charlie 文字消息互通
    console.log('   Bob 发消息...');
    const msg = `AFTER_AUTHORITY_LEFT_${Date.now()}`;
    await typeMessage(bob, msg);
    let msgOk = false;
    try {
      await charlie.waitForSelector(`text=${msg}`, { timeout: 8000 });
      msgOk = true;
    } catch (e) {}
    console.log(`   Charlie 收到 Bob 消息: ${msgOk ? '✅' : '❌'}`);

    // 5.4 Bob 能否重新共享屏幕
    console.log('   Bob 共享屏幕...');
    await bob.locator('[data-testid="media-quality"]').selectOption('720p30');
    await bob.click('[data-testid="media-share-toggle"]');
    let bobShareOk = false;
    try {
      await bob.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });
      await charlie.waitForFunction(() => {
        for (const v of document.querySelectorAll('video')) {
          if (v.videoWidth > 0) return true;
        }
        return false;
      }, { timeout: 20000 });
      bobShareOk = true;
    } catch (e) {
      console.log(`   ⚠️ Bob 共享失败: ${e.message}`);
    }
    console.log(`   Bob 能共享 + Charlie 收到: ${bobShareOk ? '✅' : '❌'}`);

    if (bobShareOk) {
      await bob.click('[data-testid="media-share-toggle"]');
      await bob.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });
    }

    // === 阶段 6：新人 Dave 加入（测试 authority 迁移后的拉人） ===
    console.log('\n👤 阶段 6: 新人 Dave 登录 + 加入语音（authority 迁移测试）');
    const dave = await (await browser.newContext()).newPage();
    await setDisableAdaptive(dave);
    await login(dave, daveName);
    console.log(`   Dave: ${daveName}`);
    await bob.waitForTimeout(5000); // 等自动拉人

    // Dave 尝试加入语音
    console.log('   Dave 加入语音...');
    let daveJoinOk = false;
    try {
      await dave.click('[data-testid="channel-item-voice"]');
    await dave.waitForTimeout(3000);
    await dave.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await dave.click('[data-testid="media-join"]');
    await dave.waitForTimeout(3000);
      await dave.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
      daveJoinOk = true;
    } catch (e) {
      console.log(`   ⚠️ Dave 加入语音失败: ${e.message}`);
    }
    console.log(`   Dave 加入语音: ${daveJoinOk ? '✅' : '❌'}`);

    // === 诊断：Dave 的 channel state ===
    let daveState = null;
    if (daveJoinOk) {
      daveState = await dave.evaluate(() => {
        const d = window.__kaede_debug__;
        return d ? d.channelState('general') : null;
      }).catch(e => ({ error: String(e) }));
      console.log('\n🔬 Dave state:', JSON.stringify(daveState, null, 2));
      if (daveState && daveState.key_b64 && bobState && bobState.key_b64) {
        console.log('   Dave vs Bob 密钥一致: ' + (daveState.key_b64 === bobState.key_b64 ? '✅' : '❌'));
        console.log('   Dave vs Bob epoch 一致: ' + (daveState.epoch === bobState.epoch ? '✅' : ('❌ (Dave=' + daveState.epoch + ', Bob=' + bobState.epoch + ')')));
      }
    }

    // Dave 是否能收到消息
    if (daveJoinOk) {
      console.log('   Bob 发第二条消息...');
      const msg2 = `DAVE_JOINED_${Date.now()}`;
      await typeMessage(bob, msg2);
      let daveGotMsg = false;
      try {
        await dave.waitForSelector(`text=${msg2}`, { timeout: 8000 });
        daveGotMsg = true;
      } catch (e) {}
      console.log(`   Dave 收到 Bob 消息: ${daveGotMsg ? '✅' : '❌'}`);
    }

    // === 汇总 ===
    console.log('\n========================================');
    console.log('  结果分析');
    console.log('========================================');
    const checks = [
      ['Bob + Charlie 互相可见', bobSeesCharlie > 0 && charlieSeesBob > 0],
      ['Bob 开麦 Charlie 看到', charlieSeesMic > 0],
      ['Bob 发消息 Charlie 收到', msgOk],
      ['Bob 能共享屏幕', bobShareOk],
      ['新人 Dave 能加入语音', daveJoinOk],
    ];
    let passed = 0;
    for (const [name, ok] of checks) {
      console.log(`   ${ok ? '✅' : '❌'} ${name}`);
      if (ok) passed++;
    }
    console.log(`\n${passed}/${checks.length} 通过`);

    if (passed === checks.length) {
      console.log('\n🎉 测试 9 通过：authority 退出后语音功能完整');
    } else {
      console.log('\n⚠️ 测试 9 部分失败：authority 退出后某些功能异常');
      process.exit(1);
    }
  } catch (e) {
    console.error('\n❌ 测试 9 失败:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
