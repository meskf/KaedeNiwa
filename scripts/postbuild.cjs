#!/usr/bin/env node
// PWA post-build：dx build 后把 assets 里 PWA 相关文件铺到 dist 根。
// 用法：node scripts/postbuild.cjs [dist 路径]
// 默认 dist：target/dx/kaede-wasm/debug/web/public

const fs = require('fs');
const path = require('path');
const ROOT = path.resolve(__dirname, '..');
const DIST = process.argv[2] || path.join(ROOT, 'target', 'dx', 'kaede-wasm', 'debug', 'web', 'public');
const ASSETS = path.join(ROOT, 'crates', 'kaede-wasm', 'assets');

if (!fs.existsSync(DIST)) {
  console.error(`❌ dist 不存在: ${DIST}`);
  process.exit(1);
}
if (!fs.existsSync(ASSETS)) {
  console.error(`❌ assets 不存在: ${ASSETS}`);
  process.exit(1);
}

const FILES = [
  "manifest.json",
  "sw.js",
  "boot.js",
  "livekit-client.umd.min.js",
  "livekit-client.e2ee.worker.js",
  "icon.svg",
  "favicon.svg",
  "favicon.ico",
  "icon-16.png",
  "icon-32.png",
  "icon-48.png",
  "icon-64.png",
  "icon-96.png",
  "icon-128.png",
  "icon-144.png",
  "icon-152.png",
  "icon-167.png",
  "icon-180.png",
  "icon-192.png",
  "icon-256.png",
  "icon-384.png",
  "icon-512.png",
  "icon-512-maskable.png",
];
for (const f of FILES) {
  const src = path.join(ASSETS, f);
  const dst = path.join(DIST, f);
  if (!fs.existsSync(src)) {
    console.log(`  ✗ 源缺失 ${f}`);
    continue;
  }
  fs.copyFileSync(src, dst);
  console.log(`  ✓ ${f}  ${fs.statSync(dst).size} bytes`);
}

// 检查 index.html 引用（SW 注册在 boot.js 里，不内联）
const indexHtml = path.join(DIST, 'index.html');
const bootJs = path.join(DIST, 'boot.js');
if (fs.existsSync(indexHtml)) {
  const html = fs.readFileSync(indexHtml, 'utf8');
  const boot = fs.existsSync(bootJs) ? fs.readFileSync(bootJs, 'utf8') : '';
  const checks = [
    ['manifest link', html.includes('rel="manifest"')],
    [
      'SW register',
      boot.includes('serviceWorker') && boot.includes('.register('),
    ],
    ['icon link', html.includes('icon-32.png')],
    ['viewport-fit', html.includes('viewport-fit=cover')],
  ];
  console.log('\n  index.html 引用检查:');
  for (const [name, ok] of checks) console.log(`    ${ok ? '✓' : '✗'} ${name}`);
}

console.log('\n✓ postbuild 完成');
