const fs = require('fs')
let en = fs.readFileSync('src/locales/en.ts', 'utf8')
const enAnchor = `  audio: {`
const enAdd = `  search: {
    title: 'Search',
    placeholder: 'Search messages, channels, people...',
    shortcutHint: 'Ctrl K',
    noResults: 'No results',
    typeToStart: 'Start typing to search',
    inChannel: 'in',
    results: 'results',
  },
  audio: {`
if (!en.includes(enAnchor)) { console.error('FAIL: en anchor'); process.exit(1) }
if (en.includes('search: {')) { console.log('SKIP en: already has search'); } else {
  en = en.replace(enAnchor, enAdd)
  fs.writeFileSync('src/locales/en.ts', en)
  console.log('OK en.ts')
}

let zh = fs.readFileSync('src/locales/zh.ts', 'utf8')
const zhAnchor = `  audio: {`
const zhAdd = `  search: {
    title: '搜索',
    placeholder: '搜索消息、频道、用户...',
    shortcutHint: 'Ctrl K',
    noResults: '无结果',
    typeToStart: '输入关键词开始搜索',
    inChannel: '在',
    results: '条结果',
  },
  audio: {`
if (zh.includes('search: {')) { console.log('SKIP zh: already has search'); } else {
  zh = zh.replace(zhAnchor, zhAdd)
  fs.writeFileSync('src/locales/zh.ts', zh)
  console.log('OK zh.ts')
}
