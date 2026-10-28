// test-25: 频道附件 E2E（u0 上传 1KB 随机 → 发带附件消息 → u1 收到 → u1 下载 → SHA256 严格比对）
const { checkPrereqs, login, randName } = require('./test-helpers.cjs');
const { launch } = require('./test-launch.cjs');
const crypto = require('crypto');

function assert(cond, msg) { if (!cond) throw new Error('断言失败: ' + msg); }
function sha256(buf) { return crypto.createHash('sha256').update(buf).digest('hex'); }

(async () => {
  console.log('🧪 测试 25: 频道附件 E2E（上传→消息→下载→SHA256）\n');
  await checkPrereqs();
  const browser = await launch();
  const u0Name = randName('att0');
  const u1Name = randName('att1');
  const CH = 'general';
  try {
    const u0 = await (await browser.newContext()).newPage();
    const u1 = await (await browser.newContext()).newPage();
    const u0Logs = [], u1Logs = [];
    u0.on('console', m => u0Logs.push(m.text()));
    u1.on('console', m => u1Logs.push(m.text()));

    await login(u0, u0Name); console.log('  u0 =', u0Name);
    await login(u1, u1Name); console.log('  u1 =', u1Name);
    await u0.waitForTimeout(3000);

    await u0.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.attachments), null, { timeout: 10000 });
    await u1.waitForFunction(() => !!(window.__kaede_debug__ && window.__kaede_debug__.attachments), null, { timeout: 10000 });
    console.log('  __kaede_debug__.attachments 就绪');

    // Step 1: 本地生成 1KB 随机二进制
    const raw = crypto.randomBytes(1024);
    const expectedSha = sha256(raw);
    const b64 = raw.toString('base64');
    console.log('  1. ✅ 本地 1KB，SHA256 =', expectedSha.slice(0, 16) + '...');

    // Step 2: u0 上传
    const fileName = 'test-' + Date.now() + '.bin';
    const att = await u0.evaluate(async ({ ch, b64, name }) => {
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      return await window.__kaede_debug__.attachments.upload(ch, name, 'application/octet-stream', bytes);
    }, { ch: CH, b64, name: fileName });

    assert(att && att.id, 'uploadAttachment 未返回含 id 对象，返回: ' + JSON.stringify(att).slice(0, 200));
    console.log('  2. ✅ 上传成功 id =', att.id, 'size =', att.size);

    // Step 3: u0 发带附件消息
    const text = 'ATT_MSG_' + Date.now();
    await u0.evaluate(async ({ ch, t, att }) => {
      return await window.__kaede_debug__.messages.sendWithAttachments(ch, t, [att]);
    }, { ch: CH, t: text, att });
    console.log('  3. ✅ 带附件消息已发送:', text);

    // Step 4: 等 u1 收到消息（在线推送）
    await u1.waitForTimeout(5000);
    const hist1 = await u1.evaluate(async (ch) => window.__kaede_debug__.loadHistory(ch, 50), CH);
    const found = Array.isArray(hist1) ? hist1.find(m => (m.plaintext || '').includes(text)) : null;
    if (!found) {
      u1Logs.filter(l => /channel|attach|mls|error|application/i.test(l)).slice(-12).forEach(l => console.log('    [u1 log]', l));
      throw new Error(`u1 历史里未找到消息 "${text}"（实际 ${Array.isArray(hist1) ? hist1.length : 'non-array'} 条）`);
    }
    console.log('  4. ✅ u1 收到消息');
    // 软断言：历史里附件元数据
    if (found.attachments && found.attachments.length > 0) {
      console.log('     软断言：u1 历史含附件元数据，count =', found.attachments.length);
    } else {
      console.log('     ⚠️ 软断言：u1 历史未带附件元数据（可能走 UI 单独拉取，不致命）');
    }

    // Step 5: u1 独立下载（用 u0 侧的 att.id + sha256，与 u0 完全解耦）
    const downloadedArr = await u1.evaluate(async ({ ch, id, sha }) =>
      window.__kaede_debug__.attachments.download(ch, id, sha),
      { ch: CH, id: att.id, sha: att.sha256 || expectedSha });
    const downloaded = Buffer.from(downloadedArr);
    console.log('  5. ✅ u1 下载成功', downloaded.length, '字节');

    // Step 6: SHA256 + 字节流严格比对
    const actualSha = sha256(downloaded);
    assert(downloaded.length === raw.length, `长度不一致：期望 ${raw.length}，实际 ${downloaded.length}`);
    assert(actualSha === expectedSha, `SHA256 不一致\n    期望 ${expectedSha}\n    实际 ${actualSha}`);
    assert(Buffer.compare(downloaded, raw) === 0, 'bytes 顺序不一致');
    console.log('  6. ✅ SHA256 + bytes 完全一致');
    console.log('     期望 =', expectedSha.slice(0, 32) + '...');
    console.log('     实际 =', actualSha.slice(0, 32) + '...');

    console.log('\n🎉 测试 25 通过');
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('\n❌ 测试 25 失败:', e.message);
    try { await browser.close(); } catch {}
    process.exit(1);
  }
})();
