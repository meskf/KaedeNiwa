const fs = require('fs')
const p = 'src/components/MainView.tsx'
const lines = fs.readFileSync(p, 'utf8').split('\n')
const log = []

// B1. mapped 加 category_id
for (let i = 0; i < lines.length; i++) {
  if (lines[i].trim() === 'is_private: !!c.is_private,') {
    lines[i] = lines[i] + '\n          category_id: c.category_id ?? null,'
    log.push('OK mapped category_id at line ' + (i + 1))
    break
  }
}

// B2. 在 loadChannels useEffect 里追加 listCategories（在 setChannelList 后）
for (let i = 0; i < lines.length; i++) {
  if (lines[i].trim() === 'setChannelList(mapped)') {
    const indent = lines[i].match(/^(\s*)/)[1]
    lines.splice(i + 1, 0,
      `${indent}try {`,
      `${indent}  const rawCats = await client.listCategories()`,
      `${indent}  const catArr = typeof rawCats === 'string' ? JSON.parse(rawCats) : rawCats`,
      `${indent}  if (Array.isArray(catArr)) setCategories(catArr)`,
      `${indent}} catch (e) { console.warn('[categories] list failed (fallback to kind split):', e) }`
    )
    log.push('OK listCategories call at line ' + (i + 1))
    break
  }
}

// B3. destructure setCategories — find the useChatStore() destructure block
for (let i = 0; i < lines.length; i++) {
  if (lines[i].includes('} = useChatStore()')) {
    // Insert before the closing line: we need setCategories as a separate selector
    // Find the line before this closing (the destructure content ends with `  } = useChatStore()`)
    // Actually easier: insert after `  } = useChatStore()` a separate selector line
    lines.splice(i + 1, 0, `  const categories = useChatStore(s => s.categories)`, `  const setCategories = useChatStore(s => s.setCategories)`)
    log.push('OK destructure categories at line ' + (i + 1))
    break
  }
}

// B4. 找 textChannels/voiceChannels 定义区，替换为 helper 函数
let startIdx = -1, endIdx = -1
for (let i = 0; i < lines.length; i++) {
  if (lines[i].trim() === "const textChannels = channelList.filter(c => c.kind === 'text')") startIdx = i
  if (lines[i].trim() === "const voiceChannels = channelList.filter(c => c.kind === 'voice')") endIdx = i
}
if (startIdx === -1 || endIdx === -1) { console.error('FAIL: textChannels/voiceChannels not found'); process.exit(1) }

const helpers = [
  `  const renderTextChannel = (ch: any) => (`,
  `    <div`,
  `      key={ch.id}`,
  `      data-testid={\`channel-item-\${ch.id}\`}`,
  `      onClick={() => { setChannel(ch.id); setActiveView('chat') }}`,
  `      className={\`px-3 py-2 mb-1 rounded-md text-[14px] cursor-pointer flex items-center gap-2.5 transition-colors \${`,
  `        currentChannel === ch.id && activeView === 'chat'`,
  `          ? 'bg-[var(--bg-active)] text-[var(--text-main)] font-medium'`,
  `          : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'`,
  `      }\`}`,
  `    >`,
  `      <IconHash className="w-4 h-4 opacity-50" /> {ch.name}`,
  `    </div>`,
  `  )`,
  ``,
  `  const renderVoiceChannel = (vc: any) => (`,
  `    <div`,
  `      key={vc.id}`,
  `      data-testid={\`channel-item-\${vc.id}\`}`,
  `      onClick={() => { setChannel(vc.id); setActiveView('chat') }}`,
  `      className={\`px-3 py-2 mb-1 rounded-md text-[14px] cursor-pointer flex flex-col gap-2 transition-colors \${`,
  `        currentChannel === vc.id && activeView === 'chat'`,
  `          ? 'bg-[var(--bg-active)] text-[var(--text-main)] font-medium'`,
  `          : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'`,
  `      }\`}`,
  `    >`,
  `      <div className="flex items-center gap-2.5">`,
  `        <IconVolume className="w-4 h-4 opacity-50" /> {vc.name}`,
  `        {inVoice && currentChannel === vc.id && media.audioBitrate > 0 && (`,
  `          <span data-testid={\`voice-bitrate-\${vc.id}\`} className="ml-auto text-[10px] font-mono px-1.5 py-0.5 rounded bg-[var(--bg-active)] text-[var(--text-muted)]">`,
  `            {media.audioBitrate}K`,
  `          </span>`,
  `        )}`,
  `      </div>`,
  `      {inVoice && currentChannel === vc.id && (`,
  `        <div className="pl-6 flex flex-col gap-1 mt-1">`,
  `          {Object.values(media.participants || {}).map((p: any) => (`,
  `            <div`,
  `              key={p.identity}`,
  `              data-participant={p.identity}`,
  `              data-mic={p.isMicrophoneEnabled ? 'on' : 'off'}`,
  `              data-speaking={p.isSpeaking ? 'on' : 'off'}`,
  `              className="flex items-center gap-2 cursor-pointer hover:bg-[var(--bg-hover)] rounded px-1 -mx-1 py-0.5 transition-colors"`,
  `              onContextMenu={(e) => handleMemberContextMenu(e, p.identity, true)}`,
  `            >`,
  `              <div className="w-5 h-5 rounded-full bg-[var(--bg-active)] flex items-center justify-center text-[10px] font-bold text-[var(--text-main)]">`,
  `                {getInitial(p.identity)}`,
  `              </div>`,
  `              <span className="text-[12px] text-[var(--text-main)]">{p.identity}</span>`,
  `            </div>`,
  `          ))}`,
  `        </div>`,
  `      )}`,
  `    </div>`,
  `  )`,
  ``,
  `  const sortedCategories = [...categories].sort((a, b) => Number(a.position) - Number(b.position))`,
]
lines.splice(startIdx, endIdx - startIdx + 1, ...helpers)
log.push('OK helpers replace at line ' + (startIdx + 1))

// B5. 替换侧栏 Text/Voice 两段为 categories 渲染
startIdx = -1; endIdx = -1
for (let i = 0; i < lines.length; i++) {
  if (lines[i].trim() === '<div className="">') startIdx = i
  if (startIdx !== -1 && lines[i].trim() === '</aside>') { endIdx = i; break }
}
if (startIdx === -1 || endIdx === -1) { console.error('FAIL: sidebar block not found'); process.exit(1) }

const indent = lines[startIdx].match(/^(\s*)/)[1]
const sidebar = [
  `${indent}<div className="overflow-y-auto">`,
  `${indent}  {categories.length === 0 ? (`,
  `${indent}    <>`,
  `${indent}      <div className="mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">`,
  `${indent}        Text Channels`,
  `${indent}      </div>`,
  `${indent}      {channelList.filter(c => c.kind === 'text').map(renderTextChannel)}`,
  `${indent}      <div className="mt-8 mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">`,
  `${indent}        Voice Channels`,
  `${indent}      </div>`,
  `${indent}      {channelList.filter(c => c.kind === 'voice').map(renderVoiceChannel)}`,
  `${indent}    </>`,
  `${indent}  ) : (`,
  `${indent}    <>`,
  `${indent}      {sortedCategories.map((cat: any) => {`,
  `${indent}        const inCat = channelList.filter(c => c.category_id === cat.id)`,
  `${indent}        if (inCat.length === 0) return null`,
  `${indent}        return (`,
  `${indent}          <div key={cat.id} className="mb-4" data-testid={\`category-\${cat.id}\`}>`,
  `${indent}            <div className="mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">{cat.name}</div>`,
  `${indent}            {inCat.map((ch: any) => ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch))}`,
  `${indent}          </div>`,
  `${indent}        )`,
  `${indent}      })}`,
  `${indent}      {(() => {`,
  `${indent}        const uncat = channelList.filter(c => !c.category_id)`,
  `${indent}        if (uncat.length === 0) return null`,
  `${indent}        return (`,
  `${indent}          <div className="mb-4">`,
  `${indent}            <div className="mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">Uncategorized</div>`,
  `${indent}            {uncat.map((ch: any) => ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch))}`,
  `${indent}          </div>`,
  `${indent}        )`,
  `${indent}      })()}`,
  `${indent}    </>`,
  `${indent}  )}`,
  `${indent}</div>`,
]
lines.splice(startIdx, endIdx - startIdx, ...sidebar)
log.push('OK sidebar replace at line ' + (startIdx + 1))

fs.writeFileSync(p, lines.join('\n'))
log.forEach(l => console.log(l))
console.log('DONE')
