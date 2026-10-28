#!/usr/bin/env node
// apps/web/e2e/run.cjs — 统一测试执行器
//
// 用法:
//   node e2e/run.cjs                    # 跑所有
//   node e2e/run.cjs --skip-media       # 跳过 media (摄像头/麦克风)
//   node e2e/run.cjs --only=friends     # 只跑 friends 分类
//   node e2e/run.cjs --only=t22         # 只跑单测
//   node e2e/run.cjs --json             # 机器可读
//
// 退出码: 0 = 全绿, 1 = 有失败

const { execFile } = require('child_process');
const fs = require('fs');
const path = require('path');

const WEB_DIR = path.resolve(__dirname, '..');

const TESTS = [
  { id: 'pq',  name: 'PQC 消息流',       file: 'test-pq-e2e.cjs',              category: 'crypto' },
  { id: 't1',  name: '自动拉人',         file: 'test-1-auto-join.cjs',         category: 'mls' },
  { id: 't2',  name: '频道隔离',         file: 'test-2-channel-isolation.cjs', category: 'mls' },
  { id: 't3',  name: '断线重连',         file: 'test-3-reconnect.cjs',         category: 'network' },
  { id: 't4',  name: '屏幕共享',         file: 'test-4-screen-share.cjs',      category: 'media',   media: true },
  { id: 't5',  name: '麦克风+降噪',      file: 'test-5-mic-rnnoise.cjs',       category: 'media',   media: true },
  { id: 't6',  name: '画质切换',         file: 'test-6-quality.cjs',           category: 'media',   media: true },
  { id: 't7',  name: '多人测试',         file: 'test-7-multi-user.cjs',        category: 'media',   media: true },
  { id: 't8',  name: '10 用户规模',      file: 'test-8-ten-users.cjs',         category: 'media',   media: true },
  { id: 't9',  name: 'Authority 退出',   file: 'test-9-authority-leave.cjs',   category: 'media',   media: true },
  { id: 't10', name: 'Commit 补发',      file: 'test-10-commit-replay.cjs',    category: 'mls' },
  { id: 't11', name: '断线重连(离线)',   file: 'test-11-reconnect-offline.cjs',category: 'network' },
  { id: 't12', name: '多 tab',           file: 'test-12-multi-tab.cjs',        category: 'session' },
  { id: 't13', name: 'pending 队列',     file: 'test-13-pending-queue.cjs',    category: 'network' },
  { id: 't14', name: 'DM wasm 桥',       file: 'test-14-dm-wasm.cjs',          category: 'dm' },
  { id: 't20', name: '接收端画质',       file: 'test-20-receiver-quality.cjs', category: 'media',   media: true },
  { id: 't21', name: '头像/资料/图标',   file: 'test-21-avatar-ui.cjs',        category: 'ui' },
  { id: 't22', name: '好友流程',         file: 'test-22-friends-flow.cjs',     category: 'friends' },
  { id: 't23', name: '分类/频道 CRUD',   file: 'test-23-channel-crud.cjs',     category: 'channels' },
  { id: 't24', name: '搜索面板',         file: 'test-24-search-palette.cjs',   category: 'ui' },
];

const GREEN = '\x1b[32m', RED = '\x1b[31m', YELLOW = '\x1b[33m', GRAY = '\x1b[90m', RESET = '\x1b[0m';

function parseArgs() {
  const args = process.argv.slice(2);
  const opts = { only: null, skipMedia: false, json: false, timeout: 600000 };
  for (const a of args) {
    if (a.startsWith('--only=')) opts.only = a.slice(7);
    else if (a === '--skip-media') opts.skipMedia = true;
    else if (a === '--json') opts.json = true;
  }
  return opts;
}

function runOne(t, opts) {
  return new Promise(resolve => {
    const filePath = path.join(WEB_DIR, t.file);
    if (!fs.existsSync(filePath)) {
      return resolve({ ...t, status: 'MISSING', dur: 0, output: 'file not found' });
    }
    const start = Date.now();
    execFile('node', [filePath], {
      cwd: WEB_DIR,
      timeout: opts.timeout,
      maxBuffer: 10 * 1024 * 1024,
    }, (err, stdout, stderr) => {
      const dur = (Date.now() - start) / 1000;
      let status;
      if (err && err.killed) status = 'TIMEOUT';
      else if (err) status = `FAIL(${err.code || 1})`;
      else status = 'PASS';
      resolve({ ...t, status, dur: +dur.toFixed(1), stdout: stdout || '', stderr: stderr || '' });
    });
  });
}

(async () => {
  const opts = parseArgs();
  let tests = TESTS;
  if (opts.only) tests = tests.filter(t => t.id === opts.only || t.category === opts.only);
  if (opts.skipMedia) tests = tests.filter(t => !t.media);

  if (tests.length === 0) {
    console.error('No tests matched filters');
    process.exit(1);
  }

  if (!opts.json) console.log(`\nRunning ${tests.length} test(s)${opts.only ? ` [only=${opts.only}]` : ''}${opts.skipMedia ? ' [skip-media]' : ''}\n`);

  const results = [];
  for (const t of tests) {
    process.stdout.write(`  [${t.id.padEnd(4)}] ${t.name.padEnd(20)} ... `);
    const r = await runOne(t, opts);
    results.push(r);
    const color = r.status === 'PASS' ? GREEN : RED;
    console.log(`${color}${r.status.padEnd(12)}${RESET} (${r.dur}s)`);
    if (r.status !== 'PASS') {
      const text = (r.stdout || '') + (r.stderr || '');
      const lines = text.split('\n').filter(Boolean).slice(-8);
      lines.forEach(l => console.log(`         ${GRAY}│ ${l.slice(0, 180)}${RESET}`));
    }
  }

  const pass = results.filter(r => r.status === 'PASS').length;
  const fail = results.length - pass;
  console.log('\n─────────────── Summary ───────────────');
  results.forEach(r => {
    const color = r.status === 'PASS' ? GREEN : RED;
    console.log(`  ${color}${r.status.padEnd(12)}${RESET}  ${r.id.padEnd(4)}  ${r.name.padEnd(20)}  ${r.dur}s`);
  });
  console.log(`\n  ${pass}/${results.length} passed${fail ? `, ${fail} failed` : ''}\n`);

  if (opts.json) {
    console.log(JSON.stringify(results.map(({ stdout, stderr, ...rest }) => rest), null, 2));
  }

  process.exit(fail ? 1 : 0);
})();
