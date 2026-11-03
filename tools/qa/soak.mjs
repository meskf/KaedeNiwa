#!/usr/bin/env node
// tools/qa/soak.mjs — L5 长稳测试
//
// 用法:
//   node tools/qa/soak.mjs --clients 2 --minutes 1 --seed 42
//   node tools/qa/soak.mjs --clients 6 --minutes 5 --seed 42
//   node tools/qa/soak.mjs --clients 10 --minutes 10 --seed 42
//
// 架构：client 按 role 分组，一半在 voice（做 screen share），一半在 general（发消息 + reaction）。
//
// 身份 & 复现性的解耦（重要）：
//   - 动作序列由 seed 决定（同 seed = 同序列，可复现）
//   - 用户身份由 runId 决定（每次运行不同，绝无撞名）
//   历史 bug：之前 name = soak-${role}-${seed}-${idx}-${rnd()}，种子化 PRNG 每次生成同一批名字，
//   第二次跑就撞上"已注册用户"，register 400，前端停在 LoginView，message-input 永远不出现。
//
// 退出码: 0 = 所有阈值通过, 1 = 有阈值越界, 2 = 环境/初始化失败

import { createRequire } from 'node:module';
import { execSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';

import { fileURLToPath as __f } from 'node:url';
const ROOT = path.resolve(path.dirname(__f(import.meta.url)), '../..');
const QA_DIR = path.join(ROOT, 'tools/qa');
const RUN_DIR = path.join(QA_DIR, '.run');
const REPORT_DIR = path.join(RUN_DIR, 'reports');
fs.mkdirSync(REPORT_DIR, { recursive: true });

const requireFromWeb = createRequire(path.join(ROOT, 'apps/web/package.json'));
const { chromium } = requireFromWeb('playwright');

function parseArgs() {
  const a = process.argv.slice(2);
  const get = (name, def) => {
    const i = a.indexOf('--' + name);
    if (i >= 0 && a[i + 1]) return a[i + 1];
    return def;
  };
  const runId = get('runId',
    Date.now().toString(36).slice(-6) + Math.floor(Math.random() * 1e6).toString(36));
  return {
    clients:   parseInt(get('clients', '10'), 10),
    minutes:   parseFloat(get('minutes', '10')),
    seed:      parseInt(get('seed', '42'), 10),
    baseUrl:   get('base', 'http://127.0.0.1:5173'),
    sampleMs:  parseInt(get('sampleMs', '5000'), 10),
    actionMs:  parseInt(get('actionMs', '4000'), 10),
    runId,
    roles: (get('roles', 'mixed') || 'mixed').toLowerCase(),
  };
}

function mulberry32(seed) {
  let t = seed >>> 0;
  return () => {
    t += 0x6D2B79F5;
    let r = t;
    r = Math.imul(r ^ (r >>> 15), r | 1);
    r ^= r + Math.imul(r ^ (r >>> 7), r | 61);
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
  };
}

function nodeRssMb() { return process.memoryUsage().rss / (1024 * 1024); }

function backendRssMb() {
  try {
    if (process.platform === 'win32') {
      const out = execSync(
        'powershell -NoProfile -Command "Get-Process kaede-server -EA SilentlyContinue | Select-Object -First 1 -ExpandProperty WorkingSet64"',
        { encoding: 'utf8', timeout: 3000 }
      ).trim();
      const bytes = parseInt(out, 10);
      return isNaN(bytes) ? null : bytes / (1024 * 1024);
    } else {
      const out = execSync('ps -C kaede-server -o rss= | head -1', { encoding: 'utf8', timeout: 3000 }).trim();
      const kb = parseInt(out, 10);
      return isNaN(kb) ? null : kb / 1024;
    }
  } catch { return null; }
}

function walMb() {
  const p = path.join(ROOT, 'kaede.db-wal');
  try { return fs.statSync(p).size / (1024 * 1024); } catch { return 0; }
}

function percentile(sorted, p) {
  if (sorted.length === 0) return null;
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * p));
  return sorted[idx];
}

async function createClient(browser, idx, cfg, rnd, role, runId) {
  const ctx = await browser.newContext({
    ignoreHTTPSErrors: true,
    permissions: ['camera', 'microphone'],
  });
  const page = await ctx.newPage();
  const name = `soak-${role}-${cfg.seed}-${idx}-${runId}`;

  const errors = [];
  page.on('pageerror', e => errors.push(String(e).slice(0, 200)));

  await page.goto(cfg.baseUrl);
  await page.getByTestId('login-id').waitFor({ state: 'visible', timeout: 30_000 });
  await page.getByTestId('login-id').fill(name);
  await page.getByTestId('login-password').fill('password123');
  const reg = page.getByTestId('login-toggle-register');
  const regVisible = await reg.isVisible().catch(() => false);
  if (regVisible) await reg.click();
  await page.getByTestId('login-submit').click();

  try {
    await page.getByTestId('message-input').waitFor({ state: 'visible', timeout: 30_000 });
  } catch (e) {
    const diag = {
      user: name,
      url: page.url(),
      hasLoginId: await page.getByTestId('login-id').isVisible().catch(() => false),
      hasMessageInput: await page.getByTestId('message-input').isVisible().catch(() => false),
      bodyHead: (await page.locator('body').innerText().catch(() => '')).slice(0, 400),
    };
    console.error(`[soak] client ${idx} message-input timeout: ${JSON.stringify(diag)}`);
    throw new Error('message-input not visible: ' + JSON.stringify(diag));
  }

  await page.waitForFunction(() => !!window.__kaede_debug__, null, { timeout: 20_000 });

  const textChannelId = await page.evaluate(async () => {
    const dbg = window.__kaede_debug__;
    const list = await dbg.channels.list();
    return list.find(c => c.kind === 'text')?.id;
  });
  if (!textChannelId) throw new Error('no text channel');

  if (role === 'media') {
    const voice = await page.evaluate(async () => {
      const dbg = window.__kaede_debug__;
      const list = await dbg.channels.list();
      return list.find(c => c.kind === 'voice');
    });
    if (!voice) throw new Error('no voice channel');
    const nodeSel = `[data-node-id="${voice.id}"]`;
    if (await page.locator(nodeSel).count() > 0) {
      await page.locator(nodeSel).first().click();
    } else {
      await page.locator('aside').first().getByText(voice.name, { exact: true }).first().click();
    }
    await page.getByTestId('media-join').waitFor({ state: 'visible', timeout: 30_000 });
    await page.getByTestId('media-join').click();
    await page.getByTestId('media-connected').waitFor({ state: 'visible', timeout: 90_000 });
  }

  return {
    idx, name, role, ctx, page, textChannelId,
    offline: false,
    errors,
    stats: {
      sharesStarted: 0, sharesFailed: 0,
      messagesSent: 0, messagesFailed: 0,
      reactions: 0,
      disconnects: 0, reconnects: 0,
      latencySamples: [],
    },
  };
}

async function doShareStart(c, rnd) {
  const p = c.page;
  const sharing = await p.getByTestId('media-share-toggle').getAttribute('data-sharing');
  if (sharing === 'on') return;
  await p.getByTestId('media-share-toggle').click();
  try {
    await p.locator('[data-testid="media-share-toggle"][data-sharing="on"]').waitFor({ timeout: 20_000 });
    c.stats.sharesStarted++;
  } catch {
    c.stats.sharesFailed++;
  }
  await new Promise(r => setTimeout(r, 1100 + rnd() * 400));
}

async function doShareStop(c) {
  const p = c.page;
  const sharing = await p.getByTestId('media-share-toggle').getAttribute('data-sharing');
  if (sharing !== 'on') return;
  await p.getByTestId('media-share-toggle').click();
  await p.locator('[data-testid="media-share-toggle"][data-sharing="off"]').waitFor({ timeout: 15_000 }).catch(() => {});
}

async function doSendMessage(c, rnd) {
  if (c.offline) return;
  const p = c.page;
  const text = `soak-${c.idx}-${Date.now()}-${Math.floor(rnd() * 1e4)}`;
  try {
    const t0 = Date.now();
    await p.getByTestId('message-input').fill(text);
    await p.getByTestId('message-input').press('Enter');
    const res = await p.evaluate(async ({ t, ch }) => {
      const dbg = window.__kaede_debug__;
      const check = async () => {
        const hist = await dbg.loadHistory(ch, 20);
        return JSON.stringify(hist).includes(t);
      };
      if (await check()) return { ok: true, tries: 1 };
      for (let i = 0; i < 10; i++) {
        await new Promise(r => setTimeout(r, 500));
        if (await check()) return { ok: true, tries: i + 2 };
      }
      return { ok: false, tries: 11 };
    }, { t: text, ch: c.textChannelId });
    const dt = Date.now() - t0;
    if (res.ok) { c.stats.messagesSent++; c.stats.latencySamples.push(dt); }
    else c.stats.messagesFailed++;
  } catch {
    c.stats.messagesFailed++;
  }
}

async function doReaction(c, rnd) {
  try {
    const ok = await c.page.evaluate(async ({ ch, emoji }) => {
      const dbg = window.__kaede_debug__;
      const hist = await dbg.loadHistory(ch, 10);
      const msg = hist.find(m => m.id && Number(m.id) > 0);
      if (!msg) return false;
      await dbg.reaction.sendById(ch, Number(msg.id), msg.ts, emoji);
      return true;
    }, { ch: c.textChannelId, emoji: rnd() > 0.5 ? '\u{1F44D}' : '\u{1F525}' });
    if (ok) c.stats.reactions++;
  } catch { /* silent */ }
}

async function doDisconnect(c, cfg) {
  if (c.offline) return;
  try {
    c.offline = true;
    await c.ctx.setOffline(true);
    c.stats.disconnects++;
    await new Promise(r => setTimeout(r, 3000 + Math.random() * 3000));
    await c.ctx.setOffline(false);
    c.stats.reconnects++;
    const waitEl = c.role === 'media' ? c.page.getByTestId('media-connected') : c.page.getByTestId('message-input');
    await waitEl.waitFor({ state: 'visible', timeout: 60_000 }).catch(() => {});
  } catch { /* ignore */ }
  c.offline = false;
}

async function randomAction(c, cfg, rnd) {
  const r = rnd();
  if (c.role === 'media') {
    if (r < 0.40) return doShareStart(c, rnd);
    if (r < 0.65) return doShareStop(c);
    if (r < 0.68) return doDisconnect(c, cfg);
    await new Promise(r => setTimeout(r, 2000 + rnd() * 2000));
    return;
  }
  if (r < 0.55) return doSendMessage(c, rnd);
  if (r < 0.78) return doReaction(c, rnd);
  if (r < 0.81) return doDisconnect(c, cfg);
  await new Promise(r => setTimeout(r, 2000 + rnd() * 2000));
}

async function main() {
  const cfg = parseArgs();
  const rnd = mulberry32(cfg.seed);
  const startedAt = Date.now();
  const deadline = startedAt + cfg.minutes * 60_000;

  console.log(`[soak] clients=${cfg.clients} minutes=${cfg.minutes} seed=${cfg.seed} runId=${cfg.runId}`);
  console.log(`[soak] baseUrl=${cfg.baseUrl}  roles=${cfg.roles}` +
    (cfg.roles === 'mixed' ? '  (mixed: half media / half chat)' : '  (dedicated)'));

  const browser = await chromium.launch({
    headless: false,
    args: [
      '--auto-select-desktop-capture-source=Entire screen',
      '--auto-select-desktop-capture-source=Screen 1',
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      '--allow-running-insecure-content',
      '--disable-features=WebRtcHideLocalIpsWithMdns',
    ],
  });

  const clients = [];
  for (let i = 0; i < cfg.clients; i++) {
    // [qa-p0b] roles=chat|media|mixed 决定 role 分配。
    // mixed 保留老的奇偶分半行为；chat/media 让纯一种角色跑长稳。
    let role;
    if (cfg.roles === 'chat') role = 'chat';
    else if (cfg.roles === 'media') role = 'media';
    else role = i % 2 === 0 ? 'media' : 'chat';
    try {
      const c = await createClient(browser, i, cfg, rnd, role, cfg.runId);
      clients.push(c);
      console.log(`[soak] client ${i} ready (${c.name}, role=${role})`);
    } catch (e) {
      console.error(`[soak] client ${i} failed: ${String(e).slice(0, 400)}`);
    }
    await new Promise(r => setTimeout(r, 500));
  }

  if (clients.length === 0) {
    console.error('[soak] no clients could be created');
    await browser.close();
    process.exit(2);
  }

  const actionLoops = clients.map(c => (async () => {
    while (Date.now() < deadline) {
      try { await randomAction(c, cfg, rnd); } catch { /* single-action failure */ }
      await new Promise(r => setTimeout(r, cfg.actionMs * (0.5 + rnd())));
    }
  })());

  const samples = [];
  const baseline = { node: nodeRssMb(), backend: backendRssMb(), wal: walMb() };
  const sampleCsvPath = path.join(REPORT_DIR, 'L5-samples.csv');
  fs.writeFileSync(sampleCsvPath,
    't_sec,node_rss_mb,backend_rss_mb,wal_mb,clients,media_clients,chat_clients,active_shares,msgs_sent,msgs_failed,shares_started,shares_failed,reactions,disconnects\n');

  const sampler = (async () => {
    while (Date.now() < deadline) {
      await new Promise(r => setTimeout(r, cfg.sampleMs));
      const tSec = Math.round((Date.now() - startedAt) / 1000);
      let activeShares = 0;
      const mediaClients = clients.filter(c => c.role === 'media');
      for (const c of mediaClients) {
        try {
          const v = await c.page.getByTestId('media-share-toggle').getAttribute('data-sharing');
          if (v === 'on') activeShares++;
        } catch { /* offline */ }
      }
      const snap = {
        t: tSec,
        node: nodeRssMb(),
        backend: backendRssMb(),
        wal: walMb(),
        clients: clients.length,
        mediaClients: mediaClients.length,
        chatClients: clients.length - mediaClients.length,
        activeShares,
        msgsSent: clients.reduce((s, c) => s + c.stats.messagesSent, 0),
        msgsFailed: clients.reduce((s, c) => s + c.stats.messagesFailed, 0),
        sharesStarted: clients.reduce((s, c) => s + c.stats.sharesStarted, 0),
        sharesFailed: clients.reduce((s, c) => s + c.stats.sharesFailed, 0),
        reactions: clients.reduce((s, c) => s + c.stats.reactions, 0),
        disconnects: clients.reduce((s, c) => s + c.stats.disconnects, 0),
      };
      samples.push(snap);
      fs.appendFileSync(sampleCsvPath,
        `${snap.t},${snap.node.toFixed(1)},${snap.backend?.toFixed(1) ?? ''},${snap.wal.toFixed(2)},${snap.clients},${snap.mediaClients},${snap.chatClients},${snap.activeShares},${snap.msgsSent},${snap.msgsFailed},${snap.sharesStarted},${snap.sharesFailed},${snap.reactions},${snap.disconnects}\n`);
      console.log(`[soak] t=${snap.t}s node=${snap.node.toFixed(0)}MB backend=${snap.backend?.toFixed(0) ?? '?'}MB wal=${snap.wal.toFixed(1)}MB active_shares=${activeShares}/${snap.mediaClients} msgs=${snap.msgsSent}/${snap.msgsSent + snap.msgsFailed} shares=${snap.sharesStarted}/${snap.sharesStarted + snap.sharesFailed}`);
    }
  })();

  await Promise.allSettled([...actionLoops, sampler]);

  for (const c of clients) {
    if (c.role === 'media') await doShareStop(c).catch(() => {});
  }

  const sum = k => clients.reduce((s, c) => s + c.stats[k], 0);
  const allLat = clients.flatMap(c => c.stats.latencySamples).sort((a, b) => a - b);
  const end = { node: nodeRssMb(), backend: backendRssMb(), wal: walMb() };

  const sharesTotal = sum('sharesStarted') + sum('sharesFailed');
  const msgsTotal = sum('messagesSent') + sum('messagesFailed');
  const shareSuccessRate = sharesTotal === 0 ? 1 : sum('sharesStarted') / sharesTotal;
  const messageSuccessRate = msgsTotal === 0 ? 1 : sum('messagesSent') / msgsTotal;
  const clientDropRate = 1 - (clients.length / cfg.clients);

  const report = {
    seed: cfg.seed,
    runId: cfg.runId,
    startedAt: new Date(startedAt).toISOString(),
    durationSec: Math.round((Date.now() - startedAt) / 1000),
    clients: {
      configured: cfg.clients, alive: clients.length, dropRate: clientDropRate,
      media: clients.filter(c => c.role === 'media').length,
      chat: clients.filter(c => c.role === 'chat').length,
    },
    rss: {
      nodeStartMb: baseline.node, nodeEndMb: end.node, nodeGrowthMb: end.node - baseline.node,
      backendStartMb: baseline.backend, backendEndMb: end.backend,
      backendGrowthMb: baseline.backend != null && end.backend != null ? end.backend - baseline.backend : null,
    },
    wal: { startMb: baseline.wal, endMb: end.wal, growthMb: end.wal - baseline.wal },
    shares: { started: sum('sharesStarted'), failed: sum('sharesFailed'), successRate: shareSuccessRate },
    messages: { sent: sum('messagesSent'), failed: sum('messagesFailed'), successRate: messageSuccessRate },
    reactions: sum('reactions'),
    disconnects: sum('disconnects'),
    latency: {
      samples: allLat.length,
      p50Ms: percentile(allLat, 0.50),
      p95Ms: percentile(allLat, 0.95),
      p99Ms: percentile(allLat, 0.99),
    },
    pageErrors: clients.reduce((s, c) => s + c.errors.length, 0),
    samples,
    thresholds: {
      nodeRssGrowthMb:    { actual: end.node - baseline.node, limit: 200, pass: (end.node - baseline.node) <= 200 },
      backendRssGrowthMb: { actual: baseline.backend != null && end.backend != null ? end.backend - baseline.backend : null, limit: 100, pass: baseline.backend == null || end.backend == null || (end.backend - baseline.backend) <= 100 },
      walGrowthMb:        { actual: end.wal - baseline.wal, limit: 50, pass: (end.wal - baseline.wal) <= 50 },
      shareSuccessRate:   { actual: shareSuccessRate, limit: 0.95, pass: shareSuccessRate >= 0.95 },
      messageSuccessRate: { actual: messageSuccessRate, limit: 0.99, pass: messageSuccessRate >= 0.99 },
      clientDropRate:     { actual: clientDropRate, limit: 0.02, pass: clientDropRate <= 0.02 },
    },
  };

  fs.writeFileSync(path.join(REPORT_DIR, 'L5.json'), JSON.stringify(report, null, 2));

  for (const c of clients) await c.ctx.close().catch(() => {});
  await browser.close();

  console.log('\n======== SOAK SUMMARY ========');
  console.log(`duration       ${report.durationSec}s`);
  console.log(`runId          ${cfg.runId}`);
  console.log(`clients        ${clients.length}/${cfg.clients} (media=${report.clients.media}, chat=${report.clients.chat})`);
  console.log(`nodeRss        ${baseline.node.toFixed(0)} -> ${end.node.toFixed(0)} MB (d ${(end.node - baseline.node).toFixed(0)})`);
  console.log(`backendRss     ${baseline.backend?.toFixed(0) ?? '?'} -> ${end.backend?.toFixed(0) ?? '?'} MB (d ${baseline.backend != null && end.backend != null ? (end.backend - baseline.backend).toFixed(0) : '?'})`);
  console.log(`wal            ${baseline.wal.toFixed(1)} -> ${end.wal.toFixed(1)} MB (d ${(end.wal - baseline.wal).toFixed(1)})`);
  console.log(`shares         ${sum('sharesStarted')} ok / ${sum('sharesFailed')} fail (${(shareSuccessRate * 100).toFixed(1)}%)`);
  console.log(`messages       ${sum('messagesSent')} ok / ${sum('messagesFailed')} fail (${(messageSuccessRate * 100).toFixed(1)}%)`);
  console.log(`reactions      ${sum('reactions')}`);
  console.log(`disconnects    ${sum('disconnects')}`);
  console.log(`latency        p50=${report.latency.p50Ms}ms p95=${report.latency.p95Ms}ms p99=${report.latency.p99Ms}ms (n=${allLat.length})`);
  console.log(`pageErrors     ${report.pageErrors}`);
  console.log('\nThresholds:');
  let failed = 0;
  for (const [k, v] of Object.entries(report.thresholds)) {
    console.log(`  ${v.pass ? 'PASS' : 'FAIL'}  ${k.padEnd(22)} actual=${v.actual} limit=${v.limit}`);
    if (!v.pass) failed++;
  }
  console.log(`\nReport: ${path.join(REPORT_DIR, 'L5.json')}`);
  console.log(`Samples: ${sampleCsvPath}`);

  process.exit(failed > 0 ? 1 : 0);
}

main().catch(e => { console.error('[soak] fatal:', e); process.exit(2); });
