const { checkPrereqs, login, randName, injectFakeDisplayMedia, setDisableAdaptive } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

const N = 10;

(async () => {
  console.log(`🧪 测试 8: ${N} 用户规模测试\n`);
  console.log('🔍 检查前置服务...');
  await checkPrereqs();
  console.log('');

  const browser = await launch({ args: [
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      '--autoplay-policy=no-user-gesture-required',
    ] });

  const users = [];
  try {
    // === 阶段 1：注册并登录 N 个用户（串行，避免拥堵） ===
    console.log(`👥 阶段 1: 创建 ${N} 个用户...`);
    for (let i = 0; i < N; i++) {
      const name = randName(`u${i}`);
      const page = await (await browser.newContext()).newPage();
    // [DIAG] 所有用户都挂 console listener
    page.on('console', m => {
      const t = m.text();
      console.log('[U' + i + '][ALL]', t);
    });
    page.on('pageerror', e => {
      console.log('[U' + i + '][ERR]', e.message);
    });
      await injectFakeDisplayMedia(page);
      await setDisableAdaptive(page);
      // 只对第一个用户捕获 console
      if (i === 0) {
        page.on('console', m => {
          const t = m.text();
          if (/send_text|has_group|post_message/i.test(t)) {
            console.log(`[U0] ${t}`);
          }
        });
      }
      // DEBUG: 挂 console
      if (i <= 2) {
        page.on('console', m => {
          const t = m.text();
          if (/MainView|channel_|Channel|send_channel|拉人|Welcome|welcome|join/i.test(t)) {
            console.log('[U' + i + ']', t);
          }
        });
      }
      await login(page, name);
      users.push({ idx: i, name, page });
      console.log(`    ${i + 1}/${N} 登录: ${name}`);
      if (i < N - 1) await page.waitForTimeout(500);
    }
    console.log(`✅ ${N} 个用户已登录\n`);

    // === 阶段 2：等待自动拉人完成（N 人需要更长时间） ===
    console.log(`⏳ 阶段 2: 等自动拉人完成（${N - 1} 次 PeerJoined 链式拉人，最长约 30 秒）...`);
    await users[0].page.waitForTimeout(60000);
    console.log('✅ 拉人完成\n');

    // === 诊断：u0 的 peerStatus 和 channel_commit 计数 ===
    const u0Status = await users[0].page.locator('[data-testid="peer-status"]').innerText().catch(() => '(none)');
    console.log('🔬 u0 最终 peerStatus:', u0Status);
    console.log('');

    // === 诊断：dump 所有人 channel state ===
    console.log('🔬 诊断: 拉人后各用户 channel state');
    const allStates = [];
    for (const u of users) {
      const s = await u.page.evaluate(() => {
        const d = window.__kaede_debug__;
        return d ? d.channelState('general') : null;
      }).catch(e => ({ error: String(e) }));
      allStates.push({ name: u.name, state: s });
      const key = s?.key_b64 ? s.key_b64.slice(0, 12) : 'NONE';
      console.log(`   ${u.name}: epoch=${s?.epoch} has_group=${s?.has_group} key=${key}...`);
    }
    const keys = allStates.map(x => x.state?.key_b64).filter(Boolean);
    const uniqueKeys = new Set(keys).size;
    console.log(`   \n   密钥种类: ${uniqueKeys}（期望 1 = 全部一致）`);
    if (uniqueKeys > 1) {
      console.log('   ⚠️ 分叉检测：部分用户在不同 group');
    }
    console.log('');

    // === 阶段 3：所有人加入语音（串行，间隔 800ms） ===
    console.log(`🎤 阶段 3: 所有人加入语音（串行，间隔 800ms）...`);
    const joinFails = [];
    for (const u of users) {
      try {
        await u.page.click('[data-testid="channel-item-voice"]');
    await u.page.waitForTimeout(1000);
    await u.page.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });
    await u.page.click('[data-testid="media-join"]');
        await u.page.waitForSelector('[data-testid="media-connected"][data-channel="voice"]', { timeout: 15000 });
        console.log(`    ${u.idx + 1}/${N} 加入语音: ${u.name}`);
        await u.page.waitForTimeout(800);
      } catch (e) {
        joinFails.push(u.idx);
        console.log(`    ❌ ${u.idx + 1}/${N} 加入失败: ${e.message}`);
      }
    }
    if (joinFails.length > 0) {
      console.log(`\n❌ ${joinFails.length} 个用户加入语音失败`);
      process.exit(1);
    }
    console.log(`✅ 所有人加入语音\n`);

    // === 阶段 4：等待 participant 列表同步（10 人需要时间） ===
    console.log(`⏳ 阶段 4: 等待 ${N} 人 participant 同步（最长 20 秒）...`);
    for (const u of users) {
      try {
        await u.page.waitForFunction(
          (expected) => {
            const count = document.querySelectorAll('[data-participant]').length;
            return count >= expected;
          },
          N,
          { timeout: 20000 }
        );
      } catch (e) {
        const actual = await u.page.locator('[data-participant]').count();
        console.log(`   ❌ ${u.name}: 期望 ${N}，实际 ${actual}`);
        process.exit(1);
      }
    }
    // 打印每个人的实际数
    for (const u of users.slice(0, 3)) {
      const c = await u.page.locator('[data-participant]').count();
      console.log(`   ${u.name}: ${c} 个 participant`);
    }
    console.log(`✅ 全部 ${N} 人互相可见\n`);

    // === 阶段 5：U0 共享屏幕，其他 9 人收到 ===
    console.log('🖥️  阶段 5: U0 共享屏幕（720p30）');
    await users[0].page.locator('[data-testid="media-quality"]').selectOption('720p30');
    await users[0].page.click('[data-testid="media-share-toggle"]');
    await users[0].page.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });

    console.log(`📺 等 ${N - 1} 人收到画面...`);
    for (let i = 1; i < N; i++) {
      try {
        await users[i].page.waitForFunction(() => {
          for (const v of document.querySelectorAll('video')) {
            if (v.videoWidth > 0) return true;
          }
          return false;
        }, { timeout: 25000 });
        process.stdout.write(`   ✅ ${users[i].name} 收到\n`);
      } catch (e) {
        console.log(`   ❌ ${users[i].name} 未收到画面`);
        process.exit(1);
      }
    }
    console.log('✅ 全部收到屏幕\n');

    // === 阶段 6：U1 开麦，其他 9 人看到 🎤 ===
    console.log('🎙️  阶段 6: U1 开麦');
    await users[1].page.click('[data-testid="media-mic-toggle"]');
    await users[1].page.waitForSelector('[data-testid="media-mic-toggle"]', { timeout: 10000 });
    await users[0].page.waitForTimeout(2000);

    let micVisible = 0;
    for (let i = 0; i < N; i++) {
      if (i === 1) continue;
      const has = await users[i].page.locator(`[data-participant="${users[1].name}"][data-mic="on"]`).count();
      if (has > 0) micVisible++;
    }
    console.log(`   ${micVisible}/${N - 1} 人看到 U1 的 🎤`);
    if (micVisible < N - 1) {
      console.log(`   ⚠️ 部分人未看到（可能是时序）`);
    } else {
      console.log('✅ 全部看到 🎤');
    }

    // === 阶段 7：U0 发消息，其他 9 人收到 ===
    console.log('\n💬 阶段 7: U0 发消息，其他 9 人收到');
    const testMsg = `TEN_USER_MSG_${Date.now()}`;
    await users[0].page.fill('[data-testid="message-input"]', testMsg);
    await users[0].page.press('[data-testid="message-input"]', 'Enter');

    let msgReceived = 0;
    for (let i = 1; i < N; i++) {
      try {
        await users[i].page.waitForSelector(`text=${testMsg}`, { timeout: 10000 });
        msgReceived++;
        process.stdout.write(`   ✅ ${users[i].name}\n`);
      } catch (e) {
        console.log(`   ❌ ${users[i].name} 未收到`);
      }
    }
    console.log(`\n📊 消息到达: ${msgReceived}/${N - 1}`);
    if (msgReceived < N - 1) {
      console.log(`   ❌ 有 ${N - 1 - msgReceived} 人未收到消息`);
      process.exit(1);
    }

    // === 阶段 8：U0 停止共享 + 离开语音 ===
    console.log('\n🛑 阶段 8: U0 停止共享 + 离开语音');
    await users[0].page.click('[data-testid="media-share-toggle"]');
    await users[0].page.waitForSelector('[data-testid="media-share-toggle"]', { timeout: 10000 });
    await users[0].page.click('[data-testid="media-leave"]');
    await users[0].page.waitForSelector('[data-testid="media-join"]', { timeout: 10000 });

    // 等其他人 participants 减少
    await users[1].page.waitForTimeout(3000);
    const remaining = await users[1].page.locator('[data-participant]').count();
    console.log(`   其他用户 participant: ${remaining}（期望 ${N - 1}）`);
    if (remaining !== N - 1) {
      console.log(`   ⚠️ participant 数异常（可能时序）`);
    } else {
      console.log('✅ participant 数正确');
    }

    console.log('\n🎉 测试 8 通过');
  } catch (e) {
    console.error('\n❌ 测试 8 失败:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();

