#!/usr/bin/env node
// tools/qa/chaos-restart.mjs — L7 服务端重启混沌
//
// 目标：模拟生产环境服务端进程被杀（OOM / 部署重启 / 崩溃），验证：
//   1. 在线客户端不会被永久卡死，能通过心跳+指数退避自动重连
//   2. 重启后同一批用户仍能互相收发消息（MLS 快照/群状态未丢）
//   3. 页面无未预期的 JS 异常
//
// 用法：
//   node tools/qa/chaos-restart.mjs --clients 4 --minutes 2 --restartAt 30
//
// 前置：tools/qa/start-env.ps1 -Force 已启动（QA 库、admin、test mode）
// 退出码：0 通过 / 1 未恢复 / 2 环境错误

import { createRequire } from 'node:module';
import { execSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const REPORT_DIR = path.join(ROOT, 'tools/qa/.run/reports');
fs.mkdirSync(REPORT_DIR, { recursive: true });

const requireFromWeb = createRequire(path.join(ROOT, 'apps/web/package.json'));
const { chromium } = requireFromWeb('playwright');

function arg(name, def) {
  const i = process.argv.indexOf('--' + name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : def;
}

const clientsN = parseInt(arg('clients', '4'), 10);
const minutes = parseFloat(arg('minutes', '2'));
const restartAtSec = parseInt(arg('restartAt', '30'), 10);
const baseUrl = arg('base', 'http://127.0.0.1:5173');
const runId = Date.now().toString(36).slice(-6);

function psKillBackend() {
  execSync(
    'pwsh -NoProfile -Command "Get-NetTCPConnection -LocalPort 18080 -State Listen -EA SilentlyContinue | ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -EA SilentlyContinue }"',
    { cwd: ROOT, stdio: 'pipe', timeout: 15_000 },
  );
}

/** 直接拉起 kaede-server（不走 start-env，避免其 cargo/端口探活的不确定性）。 */
function spawnServerDirect() {
  const exe = path.join(ROOT, 'target/debug/kaede-server.exe');
  if (!fs.existsSync(exe)) throw new Error(`missing ${exe}; build first`);
  const env = {
    ...process.env,
    KAEDE_BIND: '127.0.0.1:18080',
    KAEDE_DB: 'sqlite://tools/qa/.run/qa.db?mode=rwc',
    KAEDE_TEST_MODE: '1',
    KAEDE_ADMIN_ENABLED: '1',
    KAEDE_ADMIN_BIND: '127.0.0.1:18081',
    KAEDE_ADMIN_TOKEN: 'qa-admin-token',
    KAEDE_SKIP_LIVEKIT_ENV_CHECK: '1',
    LIVEKIT_API_KEY: 'devkey',
    LIVEKIT_API_SECRET: 'secret',
    LIVEKIT_WS_URL: 'ws://127.0.0.1:7880',
    RUST_LOG: 'info,kaede_server=debug',
  };
  const p = spawn(exe, [], { cwd: ROOT, env, detached: true, stdio: 'ignore' });
  p.unref();
}

/** 轮询 /health，返回恢复耗时（ms），失败返回 -1。 */
async function waitHealth(timeoutMs = 180_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const r = await fetch('http://127.0.0.1:18080/health', { signal: AbortSignal.timeout(3000) });
      if (r.ok) return Date.now();
    } catch { /* not up yet */ }
    await new Promise(res => setTimeout(res, 2000));
  }
  return -1;
}

async function loginClient(browser, idx) {
  const ctx = await browser.newContext({
    ignoreHTTPSErrors: true,
    permissions: ['camera', 'microphone'],
  });
  const page = await ctx.newPage();
  const name = `restart-${runId}-${idx}`;
  const errors = [];
  page.on('pageerror', e => errors.push(String(e).slice(0, 200)));

  await page.goto(baseUrl);
  await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 30_000 });
  await page.getByTestId('login-id').click();
  await page.keyboard.type(name, { delay: 15 });
  await page.getByTestId('login-password').click();
  await page.keyboard.type('password123', { delay: 15 });
  const reg = page.getByTestId('login-toggle-register');
  if (await reg.isVisible()) await reg.click();
  await page.getByTestId('login-submit').click();
  await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 30_000 });
  await page.waitForFunction(() => !!window.__kaede_debug__, null, { timeout: 20_000 });

  // 等频道 group 就绪（只读探测）
  await page.waitForFunction(async () => {
    const dbg = window.__kaede_debug__;
    const list = await dbg.channels.list();
    const ch = list.find(c => c.kind === 'text');
    if (!ch) return false;
    const st = await dbg.channelState(ch.id);
    return !!(st && st.has_group);
  }, null, { timeout: 60_000 });

  return { idx, name, ctx, page, errors };
}

async function sendAndConfirm(c, text, timeoutMs = 20_000) {
  await c.page.getByTestId('message-input').click();
  await c.page.keyboard.type(text, { delay: 5 });
  await c.page.keyboard.press('Enter');
  const row = c.page.locator('[data-testid="message-row"]').filter({ hasText: text }).first();
  await row.waitFor({ state: 'visible', timeout: timeoutMs });
}

async function main() {
  const startedAt = Date.now();
  console.log(`[restart] clients=${clientsN} minutes=${minutes} restartAt=${restartAtSec}s runId=${runId}`);

  const browser = await chromium.launch({ headless: true });
  const clients = [];
  for (let i = 0; i < clientsN; i++) {
    try {
      const c = await loginClient(browser, i);
      clients.push(c);
      console.log(`[restart] client ${i} ready (${c.name})`);
    } catch (e) {
      console.error(`[restart] client ${i} login failed: ${String(e).slice(0, 300)}`);
    }
    await new Promise(r => setTimeout(r, 400));
  }
  if (clients.length < 2) {
    console.error('[restart] not enough clients');
    await browser.close();
    process.exit(2);
  }

  // 同步屏障：每人发一条，所有人都能看到
  const sentinel = `sync-${runId}`;
  await sendAndConfirm(clients[0], sentinel);
  for (const c of clients) {
    await c.page.getByText(sentinel, { exact: false }).first()
      .waitFor({ state: 'visible', timeout: 30_000 });
  }

  const events = clients.map(c => ({ idx: c.idx, joinedBefore: 0, joinedAfter: 0 }));
  for (const e of events) {
    e.joinedBefore = await clients[e.idx].page.evaluate(
      () => (window.__kaede_debug__.events.list('Joined') || []).length,
    );
  }

  let restarted = false;
  let restartStart = 0;
  let restartEnd = 0;
  const deadline = startedAt + minutes * 60_000;

  const loops = clients.map((c, i) => (async () => {
    let n = 0;
    while (Date.now() < deadline) {
      const elapsed = (Date.now() - startedAt) / 1000;
      if (!restarted && elapsed >= restartAtSec) {
        restarted = true;
        restartStart = Date.now();
        console.log(`[restart] killing backend at t+${elapsed.toFixed(0)}s`);
        try { psKillBackend(); } catch (e) { console.warn('[restart] kill failed:', String(e).slice(0, 200)); }
        await new Promise(r => setTimeout(r, 10_000));
        console.log('[restart] starting backend...');
        spawnServerDirect();
        const up = await waitHealth(180_000);
        restartEnd = up > 0 ? up : Date.now();
        if (up < 0) console.warn('[restart] backend did not come back within 180s');
        console.log(`[restart] backend back after ${((restartEnd - restartStart) / 1000).toFixed(1)}s`);
      }
      if (restarted && Date.now() - restartEnd < 8_000) {
        await new Promise(r => setTimeout(r, 1000));
        continue;
      }
      const text = `r-${runId}-c${i}-${n++}`;
      try {
        await sendAndConfirm(c, text, 30_000);
      } catch { /* 单条失败不终止 */ }
      await new Promise(r => setTimeout(r, 1200));
    }
  })());

  await Promise.allSettled(loops);

  for (const e of events) {
    e.joinedAfter = await clients[e.idx].page.evaluate(
      () => (window.__kaede_debug__.events.list('Joined') || []).length,
    );
  }

  // 恢复后交叉验证：每人发一条 probe，所有其他人必须能看到
  const probes = clients.map((_, i) => `probe-${runId}-${i}`);
  for (let i = 0; i < clients.length; i++) {
    await sendAndConfirm(clients[i], probes[i], 60_000).catch(() => {});
  }
  let crossOk = true;
  for (let i = 0; i < clients.length; i++) {
    for (let j = 0; j < clients.length; j++) {
      if (i === j) continue;
      try {
        await clients[i].page.getByText(probes[j], { exact: false }).first()
          .waitFor({ state: 'visible', timeout: 30_000 });
      } catch {
        crossOk = false;
        console.error(`[restart] client ${i} missing probe from ${j}`);
      }
    }
  }

  const pageErrors = clients.flatMap(c => c.errors);
  const report = {
    runId,
    clients: clients.length,
    restartAtSec,
    restartDurationSec: restartEnd ? Math.round((restartEnd - restartStart) / 1000) : null,
    reconnect: events.map(e => ({ client: e.idx, joinedBefore: e.joinedBefore, joinedAfter: e.joinedAfter })),
    crossVisibleAfterRestart: crossOk,
    pageErrors,
    durationSec: Math.round((Date.now() - startedAt) / 1000),
  };
  fs.writeFileSync(path.join(REPORT_DIR, 'L7.json'), JSON.stringify(report, null, 2));

  for (const c of clients) await c.ctx.close().catch(() => {});
  await browser.close();

  console.log('\n======== RESTART CHAOS SUMMARY ========');
  console.log(`restart duration      ${report.restartDurationSec}s`);
  console.log(`reconnects            ${events.map(e => `${e.idx}:${e.joinedBefore}->${e.joinedAfter}`).join(' ')}`);
  console.log(`cross-visible         ${crossOk ? 'PASS' : 'FAIL'}`);
  console.log(`pageErrors            ${pageErrors.length}`);
  console.log(`report                ${path.join(REPORT_DIR, 'L7.json')}`);

  const reconnectHappened = events.some(e => e.joinedAfter > e.joinedBefore);
  const ok = crossOk && reconnectHappened && pageErrors.length === 0;
  process.exit(ok ? 0 : 1);
}

main().catch(e => { console.error('[restart] fatal:', e); process.exit(2); });
