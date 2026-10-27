const { checkPrereqs, login, randName, typeMessage } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');

(async () => {
  console.log('🧪 测试 10: 频道 Commit 补发（reload + 拉人触发 epoch 前进）');
  await checkPrereqs();
  const browser = await launch();
  try {
    const u0Name = randName('u0');
    const u1Name = randName('u1');
    const u2Name = randName('u2');
    const u3Name = randName('u3');

    const u0Page = await (await browser.newContext()).newPage();
    u0Page.on('console', m => {
      const t = m.text();
      if (/send_channel_commit|channel_process_commit|ChannelSync 0x05|ensure_channel|Joined|PeerJoined/.test(t)) {
        console.log('[U0]', t);
      }
    });
    await login(u0Page, u0Name);
    console.log('  u0 登录:', u0Name);

    const u1Page = await (await browser.newContext()).newPage();
    u1Page.on('console', m => {
      const t = m.text();
      if (/\/join_room|backfill_all|report_channel_epoch|report_epoch|restore|snapshot|channel_join_via_welcome|ChannelSync|MlsCommit|channel commit|channel_process_commit|process_channel_sync|send_channel_commit|Sync failed|ChannelEpochChanged|MlsSyncError|client:|Joined|PeerJoined/.test(t)) {
        console.log('[U1]', t);
      }
    });
    u1Page.on('pageerror', e => console.log('[U1][ERR]', e.message));
    await login(u1Page, u1Name);
    console.log('  u1 登录:', u1Name);

    const u2Page = await (await browser.newContext()).newPage();
    await login(u2Page, u2Name);
    console.log('  u2 登录:', u2Name);

    // 等 u0 authority 拉 u1/u2 完成 → epoch 应为 2
    await u0Page.waitForTimeout(6000);
    console.log('  ✅ 3 人已进 #general (epoch≈2)');

    const u1Before = await u1Page.evaluate(() => window.__kaede_debug__?.channelState?.('general'));
    console.log('  [probe-before] u1 epoch=' + (u1Before?.epoch) + ' has_group=' + (u1Before?.has_group));

    // 关键时序：u1 重载（页面销毁 → IndexedDB 恢复 epoch=2）
    // 并发 u3 登录触发 u0 拉人 → epoch 3 的 Commit 在 u1 恢复窗口期间产生
    // u1 恢复后要么实时收到（若 WS 就绪早），要么走 backfill_all_channel_epochs（若晚）
    const u3Page = await (await browser.newContext()).newPage();
    const u3LoginPromise = (async () => {
      await u3Page.goto('http://localhost:5173');
      await login(u3Page, u3Name);
      console.log('  u3 登录:', u3Name);
    })();
    // u1 reload 与 u3 登录并发；u1 恢复后 backfill 跑起来
    await u1Page.reload();
    await u1Page.waitForSelector('[data-testid="message-input"]', { timeout: 15000 });
    await u3LoginPromise;

    // 等 backfill + WS 稳定
    await u1Page.waitForTimeout(6000);

    // u0 发消息 → u1/u2 必须收到（能收到说明 epoch 已对齐）
    const msg = `REPLAY_${Date.now()}`;
    await typeMessage(u0Page, msg);

    const probe = async (p) => {
      try { return await p.evaluate(() => window.__kaede_debug__?.channelState?.('general')); } catch (e) { return { err: String(e) }; }
    };
    const [s0, s1, s2] = await Promise.all([probe(u0Page), probe(u1Page), probe(u2Page)]);
    console.log('  [probe] u0 epoch=' + (s0?.epoch) + ' has_group=' + (s0?.has_group));
    console.log('  [probe] u1 epoch=' + (s1?.epoch) + ' has_group=' + (s1?.has_group));
    console.log('  [probe] u2 epoch=' + (s2?.epoch) + ' has_group=' + (s2?.has_group));
    let pass = true;
    try {
      await u1Page.waitForSelector(`text=${msg}`, { timeout: 10000 });
      console.log('  ✅ u1 收到:', msg);
    } catch (e) {
      console.log('  ❌ u1 未收到:', msg);
      pass = false;
    }
    try {
      await u2Page.waitForSelector(`text=${msg}`, { timeout: 5000 });
      console.log('  ✅ u2 收到:', msg);
    } catch (e) {
      console.log('  ❌ u2 未收到:', msg);
      pass = false;
    }

    if (pass) {
      console.log('🎉 测试 10 通过：reload 后 u1 能追平 epoch 并收发消息');
    } else {
      console.log('❌ 测试 10 失败');
      process.exit(1);
    }
  } catch (e) {
    console.error('❌ 测试 10 异常:', e.message);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
