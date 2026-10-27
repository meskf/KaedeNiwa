import { useCallback, useEffect, useRef, type RefObject } from 'react'
import { LexicalComposer } from '@lexical/react/LexicalComposer'
import { RichTextPlugin } from '@lexical/react/LexicalRichTextPlugin'
import { ContentEditable } from '@lexical/react/LexicalContentEditable'
import { HistoryPlugin } from '@lexical/react/LexicalHistoryPlugin'
import { OnChangePlugin } from '@lexical/react/LexicalOnChangePlugin'
import { MarkdownShortcutPlugin } from '@lexical/react/LexicalMarkdownShortcutPlugin'
import { ListPlugin } from '@lexical/react/LexicalListPlugin'
import { LinkPlugin } from '@lexical/react/LexicalLinkPlugin'
import { LexicalErrorBoundary } from '@lexical/react/LexicalErrorBoundary'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  COMMAND_PRIORITY_HIGH,
  KEY_ENTER_COMMAND,
} from 'lexical'
import { ListNode, ListItemNode } from '@lexical/list'
import { CodeNode, CodeHighlightNode } from '@lexical/code'
import { LinkNode } from '@lexical/link'
import { QuoteNode } from '@lexical/rich-text'
import {
  BOLD_STAR,
  ITALIC_STAR,
  STRIKETHROUGH,
  INLINE_CODE,
  CODE,
  UNORDERED_LIST,
  ORDERED_LIST,
  LINK,
  QUOTE,
  type Transformer,
} from '@lexical/markdown'
import { $serializeToMarkdown } from '../lib/lexicalMarkdown'
import { IconSend } from './icons'
import { AttachmentPicker } from './AttachmentPicker'

const MD_TRANSFORMERS: Transformer[] = [
  UNORDERED_LIST,
  ORDERED_LIST,
  CODE,
  QUOTE,
  BOLD_STAR,
  ITALIC_STAR,
  STRIKETHROUGH,
  INLINE_CODE,
  LINK,
]

export interface RichTextInputHandle {
  submit: () => void
}

interface InnerProps {
  onSubmit: (md: string) => void
  placeholder: string
  handleRef?: RefObject<RichTextInputHandle | null>
}

function EditorInner({ onSubmit, placeholder, handleRef }: InnerProps) {
  const [editor] = useLexicalComposerContext()
  const markdownRef = useRef('')

  const submitNow = useCallback(() => {
    const md = markdownRef.current.trim()
    if (!md) return
    onSubmit(md)
    editor.update(() => { $getRoot().clear() })
    markdownRef.current = ''
  }, [editor, onSubmit])

  useEffect(() => {
    return editor.registerCommand(
      KEY_ENTER_COMMAND,
      (event) => {
        if (!event) return false
        if (event.shiftKey) return false
        event.preventDefault()
        submitNow()
        return true
      },
      COMMAND_PRIORITY_HIGH,
    )
  }, [editor, submitNow])

  useEffect(() => {
    if (handleRef) handleRef.current = { submit: submitNow }
  }, [handleRef, submitNow])

  useEffect(() => {
    ;(window as any).__kaede_test_set_input__ = (text: string) => {
      editor.update(() => {
        const root = $getRoot()
        root.clear()
        const p = $createParagraphNode()
        p.append($createTextNode(text))
        root.append(p)
      })
    }
    return () => { delete (window as any).__kaede_test_set_input__ }
  }, [editor])

  return (
    <div className="flex-1 flex items-center min-w-0">
      {/* 输入区：垂直居中，可多行撑高 */}
      <div className="relative flex-1 min-w-0 max-h-[200px] overflow-y-auto flex items-center py-0.5">
        <RichTextPlugin
          contentEditable={
            <ContentEditable
              data-testid="message-input"
              className="w-full bg-transparent outline-none text-[14px] text-[var(--text-main)] leading-6"
              ariaLabel={placeholder}
              spellCheck={false}
            />
          }
          placeholder={
            <div className="absolute top-0.5 left-0 text-[14px] text-[var(--text-muted)] pointer-events-none select-none leading-6">
              {placeholder}
            </div>
          }
          ErrorBoundary={LexicalErrorBoundary}
        />
        <HistoryPlugin />
        <ListPlugin />
        <LinkPlugin />
        <MarkdownShortcutPlugin transformers={MD_TRANSFORMERS} />
        <OnChangePlugin
          onChange={(state) => {
            state.read(() => {
              markdownRef.current = $serializeToMarkdown()
            })
          }}
        />
      </div>

      {/* 右侧操作区：附件、Emoji、发送，统一尺寸 h-6 对齐 */}
      <div className="flex items-center shrink-0 pl-2 h-6 gap-1">
        <div className="flex items-center justify-center w-6 h-6">
          <AttachmentPicker />
        </div>
        <button
          type="button"
          onClick={submitNow}
          className="text-[var(--text-muted)] hover:text-[var(--text-main)] transition-colors w-6 h-6 flex items-center justify-center"
          title="Send"
        >
          <IconSend className="w-4 h-4 block" />
        </button>
      </div>
    </div>
  )
}

interface Props {
  onSubmit: (md: string) => void
  placeholder?: string
  disabled?: boolean
  handleRef?: RefObject<RichTextInputHandle | null>
}

export function RichTextInput({ onSubmit, placeholder = 'Message...', disabled, handleRef }: Props) {
  if (disabled) {
    return <div className="flex-1 text-[14px] text-[var(--text-muted)] py-1">{placeholder}</div>
  }
  const initialConfig = {
    namespace: 'KaedeMessage',
    theme: {},
    onError: (e: Error) => console.error('[lexical]', e),
    nodes: [ListNode, ListItemNode, CodeNode, CodeHighlightNode, LinkNode, QuoteNode],
  }
  return (
    <LexicalComposer initialConfig={initialConfig}>
      <EditorInner onSubmit={onSubmit} placeholder={placeholder} handleRef={handleRef} />
    </LexicalComposer>
  )
}