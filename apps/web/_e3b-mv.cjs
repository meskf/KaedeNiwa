const fs = require('fs')
const p = 'src/components/MainView.tsx'
let s = fs.readFileSync(p, 'utf8')
const log = []
function rep(from, to, name) {
  if (!s.includes(from)) { log.push('MISS: ' + name); return }
  s = s.replace(from, to)
  log.push('OK: ' + name)
}

// import
rep(
  "import { SearchPalette } from './SearchPalette'",
  "import { SearchPalette } from './SearchPalette'\nimport { CreateChannelDialog } from './CreateChannelDialog'\nimport { ChannelSettingsDialog } from './ChannelSettingsDialog'",
  'import dialogs')

// state
rep(
  "  const [searchOpen, setSearchOpen] = useState(false)",
  "  const [searchOpen, setSearchOpen] = useState(false)\n  const [createDialogCategory, setCreateDialogCategory] = useState<string | null | undefined>(undefined)\n  const [settingsChannelId, setSettingsChannelId] = useState<string | null>(null)",
  'state dialogs')

// 侧栏 category header 加 + 按钮 + 频道项 hover 加 gear
// 修改 category header 渲染
rep(
  `          <div key={cat.id} className="mb-4" data-testid={\`category-\${cat.id}\`}>
            <div className="mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">{cat.name}</div>
            {inCat.map((ch: any) => ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch))}
          </div>`,
  `          <div key={cat.id} className="mb-4 group/cat" data-testid={\`category-\${cat.id}\`}>
            <div className="mb-2 flex items-center justify-between text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
              <span>{cat.name}</span>
              <button
                onClick={() => setCreateDialogCategory(cat.id)}
                className="opacity-0 group-hover/cat:opacity-100 text-[14px] leading-none hover:text-[var(--text-main)] transition-opacity"
                title={t('channel.addChannel')}
                data-testid={\`category-add-\${cat.id}\`}
              >+</button>
            </div>
            {inCat.map((ch: any) => (
              <div key={ch.id} className="group/ch relative">
                {ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch)}
                <button
                  onClick={(e) => { e.stopPropagation(); setSettingsChannelId(ch.id) }}
                  className="absolute right-2 top-2 opacity-0 group-hover/ch:opacity-100 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-opacity"
                  title={t('channel.openSettings')}
                  data-testid={\`channel-settings-\${ch.id}\`}
                >⚙</button>
              </div>
            ))}
          </div>`,
  'category header + gear')

// uncategorized 区也加 + 按钮
rep(
  `          <div className="mb-4">
            <div className="mb-2 text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">Uncategorized</div>
            {uncat.map((ch: any) => ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch))}
          </div>`,
  `          <div className="mb-4 group/cat">
            <div className="mb-2 flex items-center justify-between text-[11px] font-bold tracking-widest text-[var(--text-muted)] uppercase">
              <span>{t('channel.uncategorized')}</span>
              <button
                onClick={() => setCreateDialogCategory(null)}
                className="opacity-0 group-hover/cat:opacity-100 text-[14px] leading-none hover:text-[var(--text-main)] transition-opacity"
                title={t('channel.addChannel')}
              >+</button>
            </div>
            {uncat.map((ch: any) => (
              <div key={ch.id} className="group/ch relative">
                {ch.kind === 'voice' ? renderVoiceChannel(ch) : renderTextChannel(ch)}
                <button
                  onClick={(e) => { e.stopPropagation(); setSettingsChannelId(ch.id) }}
                  className="absolute right-2 top-2 opacity-0 group-hover/ch:opacity-100 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)] transition-opacity"
                  title={t('channel.openSettings')}
                >⚙</button>
              </div>
            ))}
          </div>`,
  'uncategorized + gear')

// fallback 场景（categories 空时）也加 gear
// 不处理——fallback 场景少见，用现有渲染

// 挂载两个 Dialog（在 SearchPalette 前）
rep(
  `      <SearchPalette`,
  `      {createDialogCategory !== undefined && (
        <CreateChannelDialog
          defaultCategoryId={createDialogCategory}
          onClose={() => setCreateDialogCategory(undefined)}
        />
      )}

      {settingsChannelId && (
        <ChannelSettingsDialog
          channelId={settingsChannelId}
          onClose={() => setSettingsChannelId(null)}
        />
      )}

      <SearchPalette`,
  'mount dialogs')

fs.writeFileSync(p, s)
log.forEach(l => console.log(l))
