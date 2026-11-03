#!/usr/bin/env node
// tools/qa/patch-p0.mjs
// P0 阻塞修复：注册限流旁路、路径去硬编码、playwright 版本锁。
// 幂等：重复跑不会重复改；--apply 前先 --dry-run 看 diff。
//
// 用法：
//   node tools/qa/patch-p0.mjs --dry-run
//   node tools/qa/patch-p0.mjs --apply

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const APPLY = process.argv.includes('--apply');
const DRY = process.argv.includes('--dry-run') || !APPLY;

const patches = [
  // ---- 1. register 限流旁路 ----
  {
    name: 'register rate-limit bypass (dev-mode + KAEDE_TEST_MODE=1)',
    file: 'crates/kaede-server/src/http.rs',
    find: /^([ \t]*)if !state\.check_rate_key\(&format!\("ip:\{ip\}:register"\), 0\.2, 10\.0\) \{$/m,
    replace: (m, indent) =>
      `${indent}// [qa-p0] dev-mode + KAEDE_TEST_MODE=1 时绕过注册 IP 限流，仅测试用。\n` +
      `${indent}// 生产构建下 cfg!(dev-mode) 恒 false，零影响。\n` +
      `${indent}let register_rate_bypassed =\n` +
      `${indent}    cfg!(feature = "dev-mode") && std::env::var("KAEDE_TEST_MODE").as_deref() == Ok("1");\n` +
      `${indent}if !register_rate_bypassed\n` +
      `${indent}    && !state.check_rate_key(&format!("ip:{ip}:register"), 0.2, 10.0)\n` +
      `${indent}{`,
    already: /register_rate_bypassed/,
    expectMatches: 1,
  },

  // ---- 2. run-all.ps1 路径 ----
  {
    name: 'run-all.ps1: $Root from $PSScriptRoot',
    file: 'tools/qa/run-all.ps1',
    find: /^\$Root = "D:\\dev2\\kaedeniwa"$/m,
    replace: '$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\\..")).Path',
    already: /\$Root = \(Resolve-Path \(Join-Path \$PSScriptRoot/,
    expectMatches: 1,
  },

  // ---- 3. run-all.ps1 npx playwright -> node_modules/.bin ----
  {
    name: 'run-all.ps1: npx playwright -> .bin (L3)',
    file: 'tools/qa/run-all.ps1',
    find: /^([ \t]*)npx playwright test --reporter=list$/m,
    replace: (m, indent) => `${indent}& "$Root\\apps\\web\\node_modules\\.bin\\playwright.cmd" test --reporter=list`,
    already: /\.bin\\playwright\.cmd" test --reporter=list/,
    expectMatches: 1,
  },
  {
    name: 'run-all.ps1: npx playwright -> .bin (L4)',
    file: 'tools/qa/run-all.ps1',
    find: /^([ \t]*)npx playwright test --config=\.\.\/\.\.\/tools\/qa\/playwright\/playwright\.l4\.config\.ts --reporter=list$/m,
    replace: (m, indent) => `${indent}& "$Root\\apps\\web\\node_modules\\.bin\\playwright.cmd" test --config=../../tools/qa/playwright/playwright.l4.config.ts --reporter=list`,
    already: /\.bin\\playwright\.cmd" test --config=\.\.\/\.\.\/tools\/qa\/playwright/,
    expectMatches: 1,
  },

  // ---- 4. stop-env.ps1 路径 ----
  {
    name: 'stop-env.ps1: $Root from $PSScriptRoot',
    file: 'tools/qa/stop-env.ps1',
    find: /^\$Root = "D:\\dev2\\kaedeniwa"$/m,
    replace: '$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\\..")).Path',
    already: /\$Root = \(Resolve-Path \(Join-Path \$PSScriptRoot/,
    expectMatches: 1,
  },

  // ---- 5. soak.mjs 路径 ----
  {
    name: 'soak.mjs: ROOT from import.meta.url',
    file: 'tools/qa/soak.mjs',
    find: /^const ROOT = 'D:\/dev2\/kaedeniwa';$/m,
    replace:
      "import { fileURLToPath as __f } from 'node:url';\n" +
      "const ROOT = path.resolve(path.dirname(__f(import.meta.url)), '../..');",
    already: /fileURLToPath as __f/,
    expectMatches: 1,
  },

  // ---- 6. .gitignore 加 .recon/ ----
  {
    name: '.gitignore: add .recon/',
    file: '.gitignore',
    find: /$/m,
    replace: (m) => `\n# QA 归档（下一轮死代码 / .bak 归档处）\n.recon/\n`,
    already: /^\.recon\/$/m,
    expectMatches: 1,
    appendOnly: true,
  },
];

let changed = 0, skipped = 0, failed = 0;
for (const p of patches) {
  const full = path.join(ROOT, p.file);
  if (!fs.existsSync(full)) {
    console.log(`[MISS] ${p.name}  (file not found: ${p.file})`);
    failed++; continue;
  }
  const src = fs.readFileSync(full, 'utf8');
  if (p.already && p.already.test(src)) {
    console.log(`[SKIP] ${p.name}  (already applied)`);
    skipped++; continue;
  }
  const matches = [...src.matchAll(new RegExp(p.find.source, p.find.flags.includes('g') ? p.find.flags : p.find.flags + 'g'))];
  if (matches.length !== p.expectMatches) {
    console.log(`[FAIL] ${p.name}  (expected ${p.expectMatches} match, got ${matches.length})`);
    failed++; continue;
  }
  let out;
  if (p.appendOnly) {
    out = src + p.replace('');
  } else {
    out = src.replace(p.find, p.replace);
  }
  if (DRY) {
    console.log(`\n[DRY] ${p.name}  ${p.file}`);
    console.log('  --- BEFORE ---');
    const m = matches[0];
    const before = src.slice(Math.max(0, m.index - 40), m.index + m[0].length + 40);
    console.log(before.split('\n').map(l => '  | ' + l).join('\n'));
    console.log('  --- AFTER  ---');
    const nm = p.appendOnly ? { index: src.length, 0: p.replace('') } : null;
    if (p.appendOnly) {
      console.log(('  | ' + p.replace('')).split('\n').map(l => '  | ' + l).join('\n'));
    } else {
      const after = out.slice(Math.max(0, matches[0].index - 40), matches[0].index + p.replace(matches[0][0], ...matches[0].slice(1)).length + 40);
      console.log(after.split('\n').map(l => '  | ' + l).join('\n'));
    }
  }
  if (APPLY) {
    fs.writeFileSync(full, out, 'utf8');
    console.log(`[OK]   ${p.name}  ${p.file}`);
  }
  changed++;
}

console.log(`\n=== summary ===`);
console.log(`  mode     : ${APPLY ? 'APPLY' : 'DRY-RUN'}`);
console.log(`  matched  : ${changed}`);
console.log(`  skipped  : ${skipped}`);
console.log(`  failed   : ${failed}`);
if (DRY) console.log(`\n看 diff 符合预期后再跑: node tools/qa/patch-p0.mjs --apply`);
process.exit(failed > 0 ? 1 : 0);
