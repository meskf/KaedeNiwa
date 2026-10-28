import { useState, useEffect } from 'react'
import { useServerStore } from '../stores/server'

// 单服务器模式：由真实数据驱动

export function StageManagerServers() {
  const profile = useServerStore(s => s.profile)

  const [mode, setMode] = useState(() => localStorage.getItem('kaede-stage-manager-mode') || 'auto')
  const [activeId, setActiveId] = useState('1')
  const [hoveredId, setHoveredId] = useState<string | null>(null)
  const [isHoveredArea, setIsHoveredArea] = useState(false)

  useEffect(() => {
    const handleToggle = () => setMode(localStorage.getItem('kaede-stage-manager-mode') || 'auto')
    window.addEventListener('kaede-stage-manager-changed', handleToggle)
    return () => window.removeEventListener('kaede-stage-manager-changed', handleToggle)
  }, [])

  if (mode === 'off') return null

  const isAuto = mode === 'auto'
  const showOut = !isAuto || isHoveredArea

  return (
    <div 
      className={`fixed left-4 top-1/2 -translate-y-1/2 z-[100] flex flex-col justify-center py-4 transition-transform duration-500 ease-out pointer-events-auto
        ${showOut ? 'translate-x-0' : '-translate-x-[80px]'}`}
      style={{ perspective: '1000px' }}
      onMouseEnter={() => isAuto && setIsHoveredArea(true)}
      onMouseLeave={() => isAuto && setIsHoveredArea(false)}
    >
      <div 
        className="flex flex-col gap-3"
        style={{
          transformOrigin: 'left center',
          transform: 'rotateY(35deg)',
          transformStyle: 'preserve-3d',
        }}
      >
        {[
    { id: 'active', type: 'server', name: profile.name },
    { id: 'add', type: 'action', name: '+', symbol: '+' },
    { id: 'more', type: 'action', name: '...', symbol: '...' },
  ].map(s => {
          const isActive = activeId === s.id
          const isHovered = hoveredId === s.id
          
          return (
            <div 
              key={s.id} 
              className="group relative flex flex-col items-center cursor-pointer"
              onClick={() => setActiveId(s.id)}
              onMouseEnter={() => setHoveredId(s.id)}
              onMouseLeave={() => setHoveredId(null)}
              style={{
                transformStyle: 'preserve-3d',
                transform: isHovered ? 'translateZ(25px) rotateY(-35deg) scale(1.05)' : 'none',
                transition: 'transform 0.3s cubic-bezier(0.25, 1, 0.5, 1)',
              }}
            >
              {/* 纯灰方块 */}
              <div 
                className={`w-[48px] h-[48px] rounded-[14px] flex items-center justify-center transition-all duration-300 border
                  ${isActive 
                    ? 'bg-zinc-500/25 border-black/10 shadow-[0_8px_16px_rgba(0,0,0,0.12)] ring-1 ring-black/10' 
                    : 'bg-zinc-500/15 border-black/5 shadow-sm opacity-80'}
                  ${isHovered ? 'opacity-100 shadow-md bg-zinc-500/20' : ''}
                `}
              >
                {/* 只有 + 和 ... 有符号，纯文字 */}
                {s.symbol && (
                  <span className="text-[18px] font-bold text-[var(--text-muted)] select-none">
                    {s.symbol}
                  </span>
                )}
              </div>

              {/* 底部文字 */}
              <span className={`mt-1.5 text-[10px] tracking-wide transition-all duration-300 ${isActive && !s.symbol ? 'text-[var(--text-main)] font-bold' : 'text-[var(--text-muted)] font-medium opacity-70'}`}>
                {s.name}
              </span>
            </div>
          )
        })}
      </div>
    </div>
  )
}