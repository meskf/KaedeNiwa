const fs = require('fs')
const path = 'src/components/MainView.tsx'
const lines = fs.readFileSync(path, 'utf8').split('\n')
// 找 {ctxMenu && (
let idx = -1
for (let i = 0; i < lines.length; i++) {
  if (lines[i].includes('{ctxMenu && (')) { idx = i; break }
}
if (idx === -1) { console.error('FAIL: {ctxMenu && ( not found'); process.exit(1) }
// 取该行前导空白
const indent = lines[idx].match(/^(\s*)/)[1]
const insert = [
  `${indent}<SearchPalette`,
  `${indent}  open={searchOpen}`,
  `${indent}  onClose={() => setSearchOpen(false)}`,
  `${indent}  onSelect={(channelId) => {`,
  `${indent}    setChannel(channelId)`,
  `${indent}    setActiveView('chat')`,
  `${indent}  }}`,
  `${indent}/>`,
  ``
]
lines.splice(idx, 0, ...insert)
fs.writeFileSync(path, lines.join('\n'))
console.log('OK: mounted SearchPalette at line', idx + 1)
