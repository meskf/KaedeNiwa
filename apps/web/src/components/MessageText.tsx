import { Fragment, type ReactNode } from 'react'

interface Props {
  text: string
  className?: string
}

interface Block {
  type: 'text' | 'codeblock'
  content: string
  lang?: string
}

export function MessageText({ text, className }: Props) {
  const blocks = splitCodeBlocks(text)
  return (
    <div className={`whitespace-pre-wrap break-words leading-relaxed ${className || ''}`}>
      {blocks.map((b, i) => {
        if (b.type === 'codeblock') {
          return (
            <pre key={i} className="my-1 p-3 rounded-md bg-[var(--bg-active)] text-[13px] overflow-x-auto">
              <code className={b.lang ? `language-${b.lang}` : undefined}>{b.content}</code>
            </pre>
          )
        }
        return <Fragment key={i}>{renderInline(b.content, 'r' + i)}</Fragment>
      })}
    </div>
  )
}

function splitCodeBlocks(text: string): Block[] {
  const blocks: Block[] = []
  const re = /```(\w*)\n?([\s\S]*?)```/g
  let last = 0
  let m: RegExpExecArray | null
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) {
      blocks.push({ type: 'text', content: text.slice(last, m.index) })
    }
    blocks.push({ type: 'codeblock', content: m[2], lang: m[1] || undefined })
    last = m.index + m[0].length
  }
  if (last < text.length) {
    blocks.push({ type: 'text', content: text.slice(last) })
  }
  if (blocks.length === 0) blocks.push({ type: 'text', content: text })
  return blocks
}

function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const out: ReactNode[] = []
  let i = 0
  let buf = ''
  let keyIdx = 0
  const pushBuf = () => {
    if (buf) {
      out.push(<Fragment key={`${keyPrefix}-t${keyIdx++}`}>{buf}</Fragment>)
      buf = ''
    }
  }

  while (i < text.length) {
    const c = text[i]
    if (c === '`') {
      const end = text.indexOf('`', i + 1)
      if (end > i) {
        pushBuf()
        out.push(
          <code key={`${keyPrefix}-c${keyIdx++}`} className="px-1 py-0.5 rounded bg-[var(--bg-active)] text-[12px] font-mono">
            {text.slice(i + 1, end)}
          </code>
        )
        i = end + 1
        continue
      }
    }
    if (c === '*' && text[i + 1] === '*') {
      const end = text.indexOf('**', i + 2)
      if (end > i) {
        pushBuf()
        const inner = text.slice(i + 2, end)
        out.push(<strong key={`${keyPrefix}-b${keyIdx++}`}>{renderInline(inner, `${keyPrefix}-bi${keyIdx}`)}</strong>)
        i = end + 2
        continue
      }
    }
    if (c === '*' && text[i + 1] !== '*') {
      const end = text.indexOf('*', i + 1)
      if (end > i) {
        pushBuf()
        const inner = text.slice(i + 1, end)
        out.push(<em key={`${keyPrefix}-i${keyIdx++}`}>{renderInline(inner, `${keyPrefix}-ii${keyIdx}`)}</em>)
        i = end + 1
        continue
      }
    }
    if (c === '~' && text[i + 1] === '~') {
      const end = text.indexOf('~~', i + 2)
      if (end > i) {
        pushBuf()
        const inner = text.slice(i + 2, end)
        out.push(<del key={`${keyPrefix}-d${keyIdx++}`}>{renderInline(inner, `${keyPrefix}-di${keyIdx}`)}</del>)
        i = end + 2
        continue
      }
    }
    if (c === '[') {
      const endText = text.indexOf(']', i + 1)
      if (endText > i && text[endText + 1] === '(') {
        const endUrl = text.indexOf(')', endText + 2)
        if (endUrl > endText) {
          const url = text.slice(endText + 2, endUrl)
          if (/^https?:\/\//i.test(url)) {
            pushBuf()
            const inner = text.slice(i + 1, endText)
            out.push(
              <a key={`${keyPrefix}-l${keyIdx++}`} href={url} target="_blank" rel="noopener noreferrer" className="text-blue-500 hover:underline">
                {inner}
              </a>
            )
            i = endUrl + 1
            continue
          }
        }
      }
    }
    buf += c
    i++
  }
  pushBuf()
  return out
}