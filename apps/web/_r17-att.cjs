const fs = require('fs')
let m = fs.readFileSync('src/components/MessageAttachments.tsx', 'utf8')
const oldBtn = `      className="mt-1 inline-flex items-center gap-2 px-3 py-2 rounded-lg bg-[var(--bg-active)] hover:bg-[var(--bg-hover)] text-left transition-colors max-w-[400px]"`
const newBtn = `      className="mt-1 inline-flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-[var(--bg-hover)] text-left transition-colors max-w-[400px]"`
if (!m.includes(oldBtn)) { console.error('FAIL'); process.exit(1) }
m = m.replace(oldBtn, newBtn)
fs.writeFileSync('src/components/MessageAttachments.tsx', m)
console.log('OK: attachment button no persistent bg')
