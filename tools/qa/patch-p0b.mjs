#!/usr/bin/env node
// tools/qa/patch-p0b.mjs —— soak.mjs 加 --roles 参数（chat|media|mixed）。
// 幂等。先 --dry-run，再 --apply。

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const FILE = path.join(ROOT, 'tools/qa/soak.mjs');
const APPLY = process.argv.includes('--apply');
const DRY = !APPLY;

let src = fs.readFileSync(FILE, 'utf8');
const ops = [];

// ---- 1. parseArgs 加 roles ----
ops.push({
  name: 'parseArgs: add roles',
  find: "    runId,\n  };\n}",
  repl:
    "    runId,\n" +
    "    roles: (get('roles', 'mixed') || 'mixed').toLowerCase(),\n" +
    "  };\n}",
  already: /roles:\s*\(get\('roles'/,
  expect: 1,
});

// ---- 2. main 里按 roles 决定 role ----
ops.push({
  name: 'main: role assignment respects cfg.roles',
  find: "    const role = i % 2 === 0 ? 'media' : 'chat';",
  repl:
    "    // [qa-p0b] roles=chat|media|mixed 决定 role 分配。\n" +
    "    // mixed 保留老的奇偶分半行为；chat/media 让纯一种角色跑长稳。\n" +
    "    let role;\n" +
    "    if (cfg.roles === 'chat') role = 'chat';\n" +
    "    else if (cfg.roles === 'media') role = 'media';\n" +
    "    else role = i % 2 === 0 ? 'media' : 'chat';",
  already: /roles === 'chat'/,
  expect: 1,
});

// ---- 3. 启动行 log 打印 roles ----
ops.push({
  name: 'startup log: show roles',
  find: "  console.log(`[soak] baseUrl=${cfg.baseUrl}  (architecture: half media / half chat)`);",
  repl:
    "  console.log(`[soak] baseUrl=${cfg.baseUrl}  roles=${cfg.roles}` +\n" +
    "    (cfg.roles === 'mixed' ? '  (mixed: half media / half chat)' : '  (dedicated)'));",
  already: /roles=\$\{cfg\.roles\}/,
  expect: 1,
});

// ---- 4. 用 roles=chat 时把 media 阈值放宽（避免 0/0 误判）----
ops.push({
  name: 'thresholds: allow media-only metrics when roles=chat',
  find: "  const clientDropRate = clients.length > 0 ? (cfg.clients - clients.length) / cfg.clients : 1;",
  repl:
    "  const clientDropRate = clients.length > 0 ? (cfg.clients - clients.length) / cfg.clients : 1;\n" +
    "  // [qa-p0b] 纯 chat / 纯 media 模式下，另一种角色的样本数为 0，\n" +
    "  // 关联率不应被判 FAIL（0/0 视为 N/A）。\n" +
    "  const mediaApplicable = cfg.roles !== 'chat';\n" +
    "  const chatApplicable  = cfg.roles !== 'media';",
  already: /mediaApplicable/,
  expect: 1,
});

let changed = 0, skipped = 0, failed = 0;
for (const op of ops) {
  if (op.already && op.already.test(src)) { console.log(`[SKIP] ${op.name}`); skipped++; continue; }
  const idx = src.indexOf(op.find);
  if (idx < 0) { console.log(`[FAIL] ${op.name}  (string not found)`); failed++; continue; }
  const count = src.split(op.find).length - 1;
  if (count !== op.expect) {
    console.log(`[FAIL] ${op.name}  (expected ${op.expect}, got ${count})`); failed++; continue;
  }
  const out = src.slice(0, idx) + op.repl + src.slice(idx + op.find.length);
  if (DRY) {
    console.log(`\n[DRY] ${op.name}`);
    console.log('  --- BEFORE ---');
    console.log(op.find.split('\n').map(l => '  | ' + l).join('\n'));
    console.log('  --- AFTER  ---');
    console.log(op.repl.split('\n').map(l => '  | ' + l).join('\n'));
  } else {
    console.log(`[OK] ${op.name}`);
  }
  src = out;
  changed++;
}
if (APPLY) fs.writeFileSync(FILE, src, 'utf8');
console.log(`\n=== summary ===  mode=${APPLY?'APPLY':'DRY-RUN'}  matched=${changed} skipped=${skipped} failed=${failed}`);
if (DRY) console.log(`确认后: node tools/qa/patch-p0b.mjs --apply`);
process.exit(failed > 0 ? 1 : 0);
