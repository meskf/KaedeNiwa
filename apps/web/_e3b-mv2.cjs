const fs = require('fs')
const path = 'src/components/MainView.tsx'
const lines = fs.readFileSync(path, 'utf8').split('\n')

// 替换 line 642-645 (index 641-644) — category 渲染
// 替换 line 652-655 (index 651-654) — uncategorized 渲染
// 从后往前替换，避免行号偏移

// 先处理 uncategorized (652-655)
const uncatOld = lines.slice(651, 655).join('\n')
if (!uncatOld.includes('Uncategorized')) {
  console.error('FAIL: uncat block mismatch at 652-655')
  console.error('actual:', JSON.stringify(uncatOld))
  process.exit(1)
}
const uncatNew = [
  '                      <div className="mb-4 group/cat">',
  '                        <div className="mb-2 flex items-center justify-between text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">',
  "                          <span>{t('channel.uncategorized')}</span>",
  '                          <button',
  '                            onClick={() => setCreateDialogCategory(null)}',
  '                            className="opacity-0 group-hover/cat:opacity-100 text-[14px] leading-none hover:text-[var(--text-main)] transition-opacity"',
  "                            title={t('channel.addChannel')}",
  '                          >+</button>',
  '                        </div>',
  '                        {uncat.map((ch: any) => (',
  '                          <div key={ch.id} className="group/ch relative">',
  "                            {ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch)}",
  '                            <button',
  '                              onClick={(e) => { e.stopPropagation(); setSettingsChannelId(ch.id) }}',
  '                              className="absolute right-2 top-2 opacity-0 group-hover/ch:opacity-100 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-opacity"',
  "                              title={t('channel.openSettings')}",
  '                            >⚙</button>',
  '                          </div>',
  '                        ))}',
  '                      </div>'
]
lines.splice(651, 4, ...uncatNew)
console.log('OK uncat replaced')

// 再处理 category (642-645，现在还是 index 641-644)
const catOld = lines.slice(641, 645).join('\n')
if (!catOld.includes('`category-${cat.id}`')) {
  console.error('FAIL: cat block mismatch at 642-645')
  console.error('actual:', JSON.stringify(catOld))
  process.exit(1)
}
const catNew = [
  '                      <div key={cat.id} className="mb-4 group/cat" data-testid={`category-${cat.id}`}>',
  '                        <div className="mb-2 flex items-center justify-between text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">',
  '                          <span>{cat.name}</span>',
  '                          <button',
  '                            onClick={() => setCreateDialogCategory(cat.id)}',
  '                            className="opacity-0 group-hover/cat:opacity-100 text-[14px] leading-none hover:text-[var(--text-main)] transition-opacity"',
  "                            title={t('channel.addChannel')}",
  '                            data-testid={`category-add-${cat.id}`}',
  '                          >+</button>',
  '                        </div>',
  '                        {inCat.map((ch: any) => (',
  '                          <div key={ch.id} className="group/ch relative">',
  "                            {ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch)}",
  '                            <button',
  '                              onClick={(e) => { e.stopPropagation(); setSettingsChannelId(ch.id) }}',
  '                              className="absolute right-2 top-2 opacity-0 group-hover/ch:opacity-100 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-opacity"',
  "                              title={t('channel.openSettings')}",
  '                              data-testid={`channel-settings-${ch.id}`}',
  '                            >⚙</button>',
  '                          </div>',
  '                        ))}',
  '                      </div>'
]
lines.splice(641, 4, ...catNew)
console.log('OK cat replaced')

fs.writeFileSync(path, lines.join('\n'))
console.log('DONE')
