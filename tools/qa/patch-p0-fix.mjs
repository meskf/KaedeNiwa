#!/usr/bin/env node
// tools/qa/patch-p0-fix.mjs —— 修补 patch-p0.mjs 漏掉的 3 项 + 追加 messages/publish_kp 限流 bypass。
// 幂等。先 --dry-run 看 diff，再 --apply。

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const APPLY = process.argv.includes('--apply');
const DRY = !APPLY;

const ops = [
  // --- 1. run-all.ps1: npx playwright -> node_modules/.bin (L3) ---
  {
    name: 'run-all.ps1 npx -> .bin (L3)',
    file: 'tools/qa/run-all.ps1',
    kind: 'regex',
    find: /^([ \t]*)npx playwright test --reporter=list[^\r\n]*$/m,
    replace: (full, indent) => `${indent}& "$Root\\apps\\web\\node_modules\\.bin\\playwright.cmd" test --reporter=list 2>&1 |`,
    already: /\.bin\\playwright\.cmd" test --reporter=list/,
    expect: 1,
  },
  // --- 2. run-all.ps1: npx playwright L4 ---
  {
    name: 'run-all.ps1 npx -> .bin (L4)',
    file: 'tools/qa/run-all.ps1',
    kind: 'regex',
    find: /^([ \t]*)npx playwright test --config=\.\.\/\.\.\/tools\/qa\/playwright\/playwright\.l4\.config\.ts[^\r\n]*$/m,
    replace: (full, indent) => `${indent}& "$Root\\apps\\web\\node_modules\\.bin\\playwright.cmd" test --config=../../tools/qa/playwright/playwright.l4.config.ts --reporter=list 2>&1 |`,
    already: /\.bin\\playwright\.cmd" test --config=\.\.\/\.\.\/tools\/qa\/playwright/,
    expect: 1,
  },
  // --- 3. .gitignore: append-only ---
  {
    name: '.gitignore append .recon/',
    file: '.gitignore',
    kind: 'append',
    text: '\n# QA 归档（死代码 / .bak 归档处）\n.recon/\n',
    already: /^\.recon\/$/m,
  },
  // --- 4. http.rs: messages rate limit bypass ---
  {
    name: 'http.rs messages rate limit bypass',
    file: 'crates/kaede-server/src/http.rs',
    kind: 'regex',
    find: /^([ \t]*)if !state\.check_rate_scoped\(&uid, "messages", 10\.0, 30\.0\) \{$/m,
    replace: (full, indent) =>
      `${indent}// [qa-p0] dev-mode + KAEDE_TEST_MODE=1 绕过 messages 限流。\n` +
      `${indent}let messages_rate_bypassed =\n` +
      `${indent}    cfg!(feature = "dev-mode") && std::env::var("KAEDE_TEST_MODE").as_deref() == Ok("1");\n` +
      `${indent}if !messages_rate_bypassed\n` +
      `${indent}    && !state.check_rate_scoped(&uid, "messages", 10.0, 30.0)\n` +
      `${indent}{`,
    already: /messages_rate_bypassed/,
    expect: 1,
  },
  // --- 5. http.rs: publish_kp rate limit bypass ---
  {
    name: 'http.rs publish_kp rate limit bypass',
    file: 'crates/kaede-server/src/http.rs',
    kind: 'regex',
    find: /^([ \t]*)if !state\.check_rate_scoped\(&uid, "publish_kp", 0\.2, 3\.0\) \{$/m,
    replace: (full, indent) =>
      `${indent}// [qa-p0] dev-mode + KAEDE_TEST_MODE=1 绕过 publish_kp 限流。\n` +
      `${indent}let publish_kp_rate_bypassed =\n` +
      `${indent}    cfg!(feature = "dev-mode") && std::env::var("KAEDE_TEST_MODE").as_deref() == Ok("1");\n` +
      `${indent}if !publish_kp_rate_bypassed\n` +
      `${indent}    && !state.check_rate_scoped(&uid, "publish_kp", 0.2, 3.0)\n` +
      `${indent}{`,
    already: /publish_kp_rate_bypassed/,
    expect: 1,
  },
];

let changed = 0, skipped = 0, failed = 0;
for (const op of ops) {
  const full = path.join(ROOT, op.file);
  if (!fs.existsSync(full)) { console.log(`[MISS] ${op.name}  (no file ${op.file})`); failed++; continue; }
  const src = fs.readFileSync(full, 'utf8');
  if (op.already && op.already.test(src)) { console.log(`[SKIP] ${op.name}`); skipped++; continue; }

  let out, m = null;
  if (op.kind === 'append') {
    out = src + op.text;
  } else {
    const re = new RegExp(op.find.source, op.find.flags.includes('g') ? op.find.flags : op.find.flags + 'g');
    const all = [...src.matchAll(re)];
    if (all.length !== op.expect) {
      console.log(`[FAIL] ${op.name}  (expected ${op.expect} match, got ${all.length})`);
      failed++; continue;
    }
    m = all[0];
    const repl = typeof op.replace === 'function' ? op.replace(m[0], ...m.slice(1)) : op.replace;
    out = src.slice(0, m.index) + repl + src.slice(m.index + m[0].length);
  }

  if (DRY) {
    console.log(`\n[DRY] ${op.name}  ${op.file}`);
    if (op.kind === 'append') {
      const tail = src.split('\n').slice(-3).join('\n');
      console.log('  --- END BEFORE ---'); console.log(tail.split('\n').map(l=>'  | '+l).join('\n'));
      console.log('  --- APPEND ---');     console.log(op.text.split('\n').map(l=>'  | '+l).join('\n'));
    } else {
      const a = src.slice(Math.max(0, m.index - 60), m.index + m[0].length + 60);
      console.log('  --- BEFORE ---'); console.log(a.split('\n').map(l=>'  | '+l).join('\n'));
      const newLen = out.length - (src.length - m[0].length);
      const b = out.slice(Math.max(0, m.index - 60), m.index + newLen + 60);
      console.log('  --- AFTER  ---'); console.log(b.split('\n').map(l=>'  | '+l).join('\n'));
    }
  }
  if (APPLY) { fs.writeFileSync(full, out, 'utf8'); console.log(`[OK] ${op.name}`); }
  changed++;
}

console.log(`\n=== summary ===  mode=${APPLY?'APPLY':'DRY-RUN'}  matched=${changed} skipped=${skipped} failed=${failed}`);
if (DRY) console.log(`确认后: node tools/qa/patch-p0-fix.mjs --apply`);
process.exit(failed > 0 ? 1 : 0);
