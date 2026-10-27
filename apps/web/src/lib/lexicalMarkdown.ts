import {
  $getRoot,
  $isElementNode,
  $isLineBreakNode,
  $isTextNode,
  type LexicalNode,
  type TextNode,
} from 'lexical'

/**
 * Lexical -> Discord 风格 Markdown 序列化。
 * 必须在 editor.getEditorState().read(() => ...) 中调用。
 */
export function $serializeToMarkdown(): string {
  const root = $getRoot()
  const blocks: string[] = []
  for (const child of root.getChildren()) {
    blocks.push(serializeBlock(child))
  }
  return blocks.join('\n').trim()
}

function serializeBlock(node: LexicalNode): string {
  if ($isTextNode(node)) return serializeText(node)
  if ($isLineBreakNode(node)) return ''
  if (!$isElementNode(node)) return ''
  const type = node.getType()
  if (type === 'list') {
    const isOrdered = (node as any).getListType?.() === 'number'
    return node.getChildren().map((item, idx) => {
      const prefix = isOrdered ? `${idx + 1}. ` : '- '
      return prefix + serializeInline(item)
    }).join('\n')
  }
  if (type === 'code') {
    const lang = (node as any).getLanguage?.() || ''
    const text = node.getTextContent()
    return '```' + lang + '\n' + text + '\n```'
  }
  if (type === 'quote') {
    return '> ' + serializeInline(node)
  }
  return serializeInline(node)
}

function serializeInline(node: LexicalNode): string {
  if ($isTextNode(node)) return serializeText(node)
  if ($isLineBreakNode(node)) return '\n'
  if (!$isElementNode(node)) return ''
  return node.getChildren().map(c => serializeInline(c)).join('')
}

function serializeText(node: TextNode): string {
  let text = node.getTextContent()
  const format = node.getFormat()
  if (format & 16) text = '`' + text + '`'
  if (format & 1) text = '**' + text + '**'
  if (format & 2) text = '*' + text + '*'
  if (format & 4) text = '~~' + text + '~~'
  return text
}