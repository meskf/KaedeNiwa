#!/usr/bin/env node
import sharp from 'sharp';
import pngToIco from 'png-to-ico';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..', '..');
const SRC = 'C:\\Users\\mes\\Pictures\\楓葉荻花\\楓葉.png';
const WASM = path.join(ROOT, 'crates', 'kaede-wasm', 'assets');
const DESK = path.join(ROOT, 'crates', 'kaede-desktop', 'assets');
const PWA_SIZES = [16, 32, 48, 64, 96, 128, 144, 152, 167, 180, 192, 256, 384, 512];

async function main() {
  if (!await fs.access(SRC).then(() => true).catch(() => false)) {
    throw new Error('源文件不存在: ' + SRC);
  }
  const meta = await sharp(SRC).metadata();
  console.log('源: ' + SRC);
  console.log('  ' + meta.width + 'x' + meta.height + '  ' + meta.format + '  alpha=' + meta.hasAlpha);
  console.log('');

  console.log('生成 PWA 图标:');
  for (const size of PWA_SIZES) {
    const out = path.join(WASM, 'icon-' + size + '.png');
    await sharp(SRC).resize(size, size, { fit: 'cover', position: 'center' })
      .png({ compressionLevel: 9 }).toFile(out);
    const st = await fs.stat(out);
    console.log('  ok  icon-' + size + '.png  ' + (st.size / 1024).toFixed(1) + ' KB');
  }

  const MS = 512, MP = Math.round(MS * 0.1), inner = MS - 2 * MP;
  const innerBuf = await sharp(SRC).resize(inner, inner, { fit: 'cover' }).png().toBuffer();
  await sharp({
    create: { width: MS, height: MS, channels: 4,
      background: { r: 0x1a, g: 0x1a, b: 0x2e, alpha: 1 } },
  }).composite([{ input: innerBuf, top: MP, left: MP }])
    .png({ compressionLevel: 9 }).toFile(path.join(WASM, 'icon-512-maskable.png'));
  console.log('  ok  icon-512-maskable.png');
  console.log('');

  console.log('生成 favicon.ico (16/32/48):');
  const favBufs = [];
  for (const s of [16, 32, 48]) {
    favBufs.push(await sharp(SRC).resize(s, s, { fit: 'cover' }).png().toBuffer());
  }
  const favIco = await pngToIco(favBufs);
  await fs.writeFile(path.join(WASM, 'favicon.ico'), favIco);
  console.log('  ok  crates/kaede-wasm/assets/favicon.ico  ' + (favIco.length / 1024).toFixed(1) + ' KB');
  console.log('');

  console.log('生成 Desktop icon.ico (16/32/48/64/128/256):');
  const deskBufs = [];
  for (const s of [16, 32, 48, 64, 128, 256]) {
    deskBufs.push(await sharp(SRC).resize(s, s, { fit: 'cover' }).png().toBuffer());
  }
  const deskIco = await pngToIco(deskBufs);
  await fs.writeFile(path.join(DESK, 'icon.ico'), deskIco);
  console.log('  ok  crates/kaede-desktop/assets/icon.ico  ' + (deskIco.length / 1024).toFixed(1) + ' KB');
  console.log('');
  console.log('ALL DONE');
}

main().catch(e => { console.error('FAIL: ' + e.message); process.exit(1); });
