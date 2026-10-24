import { promises as fs } from 'node:fs';
const files = process.argv.slice(2);
for (const f of files) {
  const buf = await fs.readFile(f);
  if (buf[0] !== 0 || buf[1] !== 0 || buf[2] !== 1 || buf[3] !== 0) {
    console.log(f + ': 不是 ICO'); continue;
  }
  const n = buf.readUInt16LE(4);
  const sizes = [];
  for (let i = 0; i < n; i++) {
    const off = 6 + i * 16;
    sizes.push((buf[off] || 256) + 'x' + (buf[off + 1] || 256));
  }
  console.log('  ' + f.split(/[\\\/]/).pop() + ': ' + n + ' 尺寸 [' + sizes.join(', ') + ']');
}
