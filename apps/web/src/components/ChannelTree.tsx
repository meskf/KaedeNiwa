import { useState, useMemo } from 'react'
import { useChatStore } from '../stores/chat'
import { IconHash, IconFolder, IconFolderOpen, IconVolume } from './icons'

export interface NodeItem {
  id: string
  name: string
  parent_id: string | null
  kind: string
  position: number
  is_private: boolean
  created_at: number
}

interface TreeNodeProps {
  node: NodeItem
  byParent: Map<string, NodeItem[]>
  depth: number
  onContextMenu: (e: React.MouseEvent, node: NodeItem) => void
  onChannelSelect?: () => void
}

function TreeNode({ node, byParent, depth, onContextMenu, onChannelSelect }: TreeNodeProps) {
  const [expanded, setExpanded] = useState(true)
  const currentChannel = useChatStore(s => s.currentChannel)
  const setChannel = useChatStore(s => s.setChannel)
  const setActiveView = useChatStore(s => s.setActiveView)

  const children = byParent.get(node.id) ?? []
  const isFolder = node.kind === 'folder'
  const isActive = currentChannel === node.id

  const handleClick = () => {
    if (isFolder) {
      setExpanded(!expanded)
    } else {
      setChannel(node.id)
      setActiveView('chat')
      onChannelSelect?.()
    }
  }

  const getIcon = () => {
    if (isFolder) return expanded ? <IconFolderOpen className="w-4 h-4 inline text-[var(--text-muted)]" /> : <IconFolder className="w-4 h-4 inline text-[var(--text-muted)]" />
    if (node.kind === 'voice') return <IconVolume className="w-4 h-4 inline text-[var(--text-muted)]" />
    return <IconHash className="w-4 h-4 inline text-[var(--text-muted)]" />
  }

  return (
    <div className="flex flex-col w-full select-none">
      <div
        data-node-id={node.id}
        onClick={handleClick}
        onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e, node) }}
        className={`group flex items-center gap-1.5 px-2 py-1.5 rounded-md cursor-pointer transition-colors ${isActive && !isFolder ? 'bg-[var(--bg-active)] text-[var(--text-main)]' : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'}`}
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
      >
        <span className="w-5 flex justify-center shrink-0">{getIcon()}</span>
        <span className="text-[14px] truncate flex-1">{node.name}</span>
      </div>
      
      {isFolder && expanded && children.length > 0 && (
        <div className="flex flex-col w-full">
          {children.map(child => (
            <TreeNode
              key={child.id}
              node={child}
              byParent={byParent}
              depth={depth + 1}
              onContextMenu={onContextMenu}
              onChannelSelect={onChannelSelect}
            />
          ))}
        </div>
      )}
    </div>
  )
}

export function ChannelTree({ onContextMenu, onChannelSelect }: {
  onContextMenu: (e: React.MouseEvent, node: NodeItem | null) => void
  onChannelSelect?: () => void
}) {
  const nodes = useChatStore(s => s.nodes)
  
  // 渲染根节点（parent_id 为 null）
  const byParent = useMemo(() => {
    const m = new Map<string, NodeItem[]>()
    for (const n of nodes) {
      const k = n.parent_id ?? '__root__'
      let arr = m.get(k)
      if (!arr) { arr = []; m.set(k, arr) }
      arr.push(n)
    }
    for (const arr of m.values()) arr.sort((a, b) => a.position - b.position)
    return m
  }, [nodes])
  const rootNodes = byParent.get('__root__') ?? []

  return (
    <div className="flex flex-col w-full gap-0.5 min-h-[200px]" onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, null) }}>
      {rootNodes.map(node => (
        <TreeNode key={node.id} node={node} byParent={byParent} depth={0} onContextMenu={onContextMenu} onChannelSelect={onChannelSelect} />
      ))}
      
    </div>
  )
}
