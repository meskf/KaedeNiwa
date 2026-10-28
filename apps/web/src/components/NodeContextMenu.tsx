import { useEffect, useRef, useState } from 'react'
import { getClient } from '../wasm/client'
import { useChatStore } from '../stores/chat'
import type { NodeItem } from './ChannelTree'
import { IconFolder, IconHash, IconVolume, IconEdit, IconTrash } from './icons'

interface Props {
  x: number
  y: number
  node: NodeItem | null
  onClose: () => void
}

export function NodeContextMenu({ x, y, node, onClose }: Props) {
  const setNodes = useChatStore(s => s.setNodes)
  const ref = useRef<HTMLDivElement>(null)
  const [mode, setMode] = useState<'main' | 'rename' | 'create-folder' | 'create-text' | 'create-voice'>('main')
  const [input, setInput] = useState('')

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', handleClick)
    return () => document.removeEventListener('mousedown', handleClick)
  }, [onClose])

  const refreshNodes = async () => {
    const c = getClient()
    if (!c) return
    const res = await (c as any).listNodes()
    const parsed = res.map((r: string) => JSON.parse(r))
    setNodes(parsed)
  }

  const handleCreate = async () => {
    if (!input.trim()) return
    const c = getClient()
    if (!c) return
    let kind = 'folder'
    if (mode === 'create-text') kind = 'text'
    if (mode === 'create-voice') kind = 'voice'
    
    try {
      await (c as any).createNode(input.trim(), kind, node ? node.id : null, false, null)
      await refreshNodes()
      onClose()
    } catch (e) {
      alert('创建失败: ' + String(e))
    }
  }

  const handleRename = async () => {
    if (!input.trim() || !node) return
    const c = getClient()
    if (!c) return
    try {
      await (c as any).updateNode(node.id, input.trim(), null, null)
      await refreshNodes()
      onClose()
    } catch (e) {
      alert('重命名失败: ' + String(e))
    }
  }

  const handleDelete = async () => {
    if (!node) return
    if (!window.confirm('确定删除该目录/频道及其所有子项吗？')) return
    const c = getClient()
    if (!c) return
    try {
      // 级联删除前端处理：从后往前删（确保子节点先被删，或者由服务端 ON DELETE CASCADE 处理）
      // 这里直接删当前节点，SQLite 的 CASCADE 会处理子节点
      await (c as any).deleteNode(node.id)
      await refreshNodes()
      onClose()
    } catch (e) {
      alert('删除失败: ' + String(e))
    }
  }

  return (
    <div
      ref={ref}
      className="fixed z-[100] w-48 bg-[var(--bg-app)] border border-[var(--bg-active)] rounded-md shadow-xl p-1 text-[13px]"
      style={{ left: Math.min(x, window.innerWidth - 200), top: Math.min(y, window.innerHeight - 250) }}
    >
      {mode === 'main' && (
        <>
          <div className="px-2 py-1.5 text-[11px] uppercase tracking-widest text-[var(--text-muted)] font-bold">节点操作</div>
          <button data-testid="ctx-create-folder" onClick={() => setMode('create-folder')} className="w-full flex items-center gap-2 px-2 py-1.5 rounded hover:bg-[var(--bg-hover)]"><IconFolder className="w-4 h-4" /> 新建子文件夹</button>
          <button data-testid="ctx-create-text" onClick={() => setMode('create-text')} className="w-full flex items-center gap-2 px-2 py-1.5 rounded hover:bg-[var(--bg-hover)]"><IconHash className="w-4 h-4" /> 新建文字频道</button>
          <button data-testid="ctx-create-voice" onClick={() => setMode('create-voice')} className="w-full flex items-center gap-2 px-2 py-1.5 rounded hover:bg-[var(--bg-hover)]"><IconVolume className="w-4 h-4" /> 新建语音频道</button>
          {node && (
            <>
              <div className="h-px bg-[var(--bg-hover)] my-1" />
              <button data-testid="ctx-rename" onClick={() => { setInput(node.name); setMode('rename') }} className="w-full flex items-center gap-2 px-2 py-1.5 rounded hover:bg-[var(--bg-hover)]"><IconEdit className="w-4 h-4" /> 重命名</button>
              <button data-testid="ctx-delete" onClick={handleDelete} className="w-full flex items-center gap-2 px-2 py-1.5 rounded hover:bg-[var(--bg-hover)] text-red-400"><IconTrash className="w-4 h-4" /> 删除</button>
            </>
          )}
        </>
      )}
      
      {mode !== 'main' && (
        <div className="flex flex-col gap-2 p-1">
          <input
            autoFocus
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter') {
                if (mode === 'rename') handleRename()
                else handleCreate()
              }
            }}
            placeholder="输入名称..."
            className="w-full bg-[var(--bg-hover)] px-2 py-1.5 rounded outline-none border border-transparent focus:border-[var(--text-muted)]"
          />
          <div className="flex justify-end gap-1">
            <button onClick={() => setMode('main')} className="px-2 py-1 rounded text-[12px] text-[var(--text-muted)] hover:text-[var(--text-main)]">取消</button>
            <button data-testid="ctx-confirm" onClick={mode === 'rename' ? handleRename : handleCreate} className="px-2 py-1 rounded text-[12px] bg-[var(--text-main)] text-[var(--bg-app)] font-semibold">确认</button>
          </div>
        </div>
      )}
    </div>
  )
}
