import { useEffect, useRef, useState } from 'react'

export function DynamicBackground() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [theme, setTheme] = useState(() => document.documentElement.getAttribute('data-theme') || 'dark')

  useEffect(() => {
    const observer = new MutationObserver(() => {
      setTheme(document.documentElement.getAttribute('data-theme') || 'dark')
    })
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d', { alpha: false })
    if (!ctx) return

    let W = 0, H = 0, dpr = 1
    let tris: any[] = []
    let cell = 100
    const INSET = 0.90
    let lightX = 0, lightY = 0
    let lightInit = false
    const pointer = { x: 0, y: 0, active: false }

    const rand = (seed: number) => {
      const s = Math.sin(seed * 12.9898) * 43758.5453;
      return s - Math.floor(s);
    }

    const makeTri = (pts: number[][], seed: number) => {
      const cx = (pts[0][0] + pts[1][0] + pts[2][0]) / 3
      const cy = (pts[0][1] + pts[1][1] + pts[2][1]) / 3
      const cross = (pts[1][0] - pts[0][0]) * (pts[2][1] - pts[0][1]) - (pts[1][1] - pts[0][1]) * (pts[2][0] - pts[0][0])
      const inner = pts.map(p => [cx + (p[0] - cx) * INSET, cy + (p[1] - cy) * INSET])
      return {
        pts: inner, cx, cy, side: cross > 0 ? 1 : 0,
        noise: rand(seed * 1.7), phase: rand(seed * 3.3) * Math.PI * 2
      }
    }

    const buildGrid = () => {
      tris = []
      cell = Math.max(64, Math.min(130, Math.min(W, H) / 7))
      const cols = Math.ceil(W / cell) + 1
      const rows = Math.ceil(H / cell) + 1
      for (let i = 0; i < rows; i++) {
        for (let j = 0; j < cols; j++) {
          const x0 = j * cell, y0 = i * cell, x1 = x0 + cell, y1 = y0 + cell
          const A = [x0, y0], B = [x1, y0], C = [x0, y1], D = [x1, y1]
          const flip = (i + j) % 2 === 0
          const seedBase = i * cols + j
          if (flip) {
            tris.push(makeTri([A, B, C], seedBase + 0.1))
            tris.push(makeTri([B, D, C], seedBase + 0.6))
          } else {
            tris.push(makeTri([A, B, D], seedBase + 0.2))
            tris.push(makeTri([A, D, C], seedBase + 0.7))
          }
        }
      }
    }

    const resize = () => {
      dpr = Math.min(window.devicePixelRatio || 1, 2)
      W = window.innerWidth
      H = window.innerHeight
      canvas.width = Math.round(W * dpr)
      canvas.height = Math.round(H * dpr)
      canvas.style.width = W + 'px'
      canvas.style.height = H + 'px'
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
      buildGrid()
      if (!lightInit) {
        lightX = W / 2
        lightY = H / 2
        lightInit = true
      }
    }

    const isDark = theme === 'dark'

    const draw = (t: number) => {
      const autoX = W * (0.5 + 0.30 * Math.sin(t * 0.00019) + 0.10 * Math.sin(t * 0.00053))
      const autoY = H * (0.5 + 0.28 * Math.cos(t * 0.00023) + 0.09 * Math.cos(t * 0.00071))
      let tx = autoX, ty = autoY
      if (pointer.active) {
        tx = tx * 0.30 + pointer.x * 0.70
        ty = ty * 0.30 + pointer.y * 0.70
      }
      lightX += (tx - lightX) * 0.055
      lightY += (ty - lightY) * 0.055

      ctx.fillStyle = isDark ? '#04060b' : '#ffffff'
      ctx.fillRect(0, 0, W, H)

      const R = Math.max(W, H) * 0.85

      const g1 = ctx.createRadialGradient(lightX, lightY, 0, lightX, lightY, R)
      if (isDark) {
        g1.addColorStop(0.00, 'rgba(160, 240, 255, 1)')
        g1.addColorStop(0.16, 'rgba(80, 180, 255, 0.78)')
        g1.addColorStop(0.42, 'rgba(38, 92, 210, 0.32)')
        g1.addColorStop(1.00, 'rgba(6, 12, 40, 0)')
      } else {
        g1.addColorStop(0.00, 'rgba(108, 176, 255, 0.58)')
        g1.addColorStop(0.16, 'rgba(140, 196, 255, 0.40)')
        g1.addColorStop(0.42, 'rgba(186, 216, 255, 0.20)')
        g1.addColorStop(0.72, 'rgba(228, 238, 252, 0.07)')
        g1.addColorStop(1.00, 'rgba(255, 255, 255, 0)')
      }
      ctx.fillStyle = g1
      ctx.fillRect(0, 0, W, H)

      const bx = W * (0.5 - 0.33 * Math.sin(t * 0.00014 + 1.2))
      const by = H * (0.5 + 0.33 * Math.cos(t * 0.00017 + 0.6))
      const g2 = ctx.createRadialGradient(bx, by, 0, bx, by, R * 0.72)
      if (isDark) {
        g2.addColorStop(0, 'rgba(170, 90, 255, 0.30)')
        g2.addColorStop(1, 'rgba(60, 0, 140, 0)')
        ctx.globalCompositeOperation = 'lighter'
      } else {
        g2.addColorStop(0.00, 'rgba(186, 152, 255, 0.32)')
        g2.addColorStop(0.45, 'rgba(212, 198, 255, 0.12)')
        g2.addColorStop(1.00, 'rgba(255, 255, 255, 0)')
      }
      ctx.fillStyle = g2
      ctx.fillRect(0, 0, W, H)
      ctx.globalCompositeOperation = 'source-over'

      if (!isDark) {
        const wx = W * (0.5 + 0.30 * Math.sin(t * 0.00011 + 3.1))
        const wy = H * (0.5 - 0.30 * Math.cos(t * 0.00013 + 2.0))
        const g3 = ctx.createRadialGradient(wx, wy, 0, wx, wy, R * 0.60)
        g3.addColorStop(0.00, 'rgba(255, 206, 176, 0.22)')
        g3.addColorStop(1.00, 'rgba(255, 255, 255, 0)')
        ctx.fillStyle = g3
        ctx.fillRect(0, 0, W, H)
      }

      ctx.lineWidth = 1

      for (let i = 0; i < tris.length; i++) {
        const tr = tris[i]
        const dx = tr.cx - lightX
        const dy = tr.cy - lightY
        const dist = Math.sqrt(dx * dx + dy * dy)
        let f = 1 - dist / R
        if (f < 0) f = 0
        f = Math.pow(f, isDark ? 2 : 1.6)
        const pulse = 0.5 + 0.5 * Math.sin(t * (isDark ? 0.0015 : 0.0012) + tr.phase)

        const p = tr.pts
        ctx.beginPath()
        ctx.moveTo(p[0][0], p[0][1])
        ctx.lineTo(p[1][0], p[1][1])
        ctx.lineTo(p[2][0], p[2][1])
        ctx.closePath()

        if (isDark) {
          const lum = 4.5 + f * 30 + pulse * 2.2 + tr.noise * 3.5
          const sat = 28 + f * 32
          const hue = tr.side ? 210 : 226
          ctx.fillStyle = 'hsl(' + hue + ', ' + sat + '%, ' + lum + '%)'
          ctx.fill()
          ctx.strokeStyle = 'hsla(192, 100%, ' + (72 + f * 20) + '%, ' + (0.05 + f * 0.42) + ')'
        } else {
          const alpha = 0.90 - f * 0.20 - tr.noise * 0.05
          const lum = 100 - (1 - f) * 3.0 - tr.noise * 1.2
          const sat = 5 + (1 - f) * 12 + pulse * 1.5
          const hue = tr.side ? 210 : 232
          ctx.fillStyle = 'hsla(' + hue + ', ' + sat + '%, ' + lum + '%, ' + alpha + ')'
          ctx.fill()
          ctx.strokeStyle = 'hsla(214, 45%, 70%, ' + (0.04 + (1 - f) * 0.11) + ')'
        }
        ctx.stroke()
      }
    }

    let startTime = 0
    let animFrame: number
    const loop = (now: number) => {
      if (!startTime) startTime = now
      draw(now - startTime)
      animFrame = requestAnimationFrame(loop)
    }

    window.addEventListener('resize', resize, { passive: true })
    const onPointerMove = (e: PointerEvent) => {
      pointer.x = e.clientX
      pointer.y = e.clientY
      pointer.active = true
    }
    const onPointerLeave = () => { pointer.active = false }
    window.addEventListener('pointermove', onPointerMove, { passive: true })
    window.addEventListener('pointerleave', onPointerLeave)

    resize()
    animFrame = requestAnimationFrame(loop)

    return () => {
      window.removeEventListener('resize', resize)
      window.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('pointerleave', onPointerLeave)
      cancelAnimationFrame(animFrame)
    }
  }, [theme])

  return (
    <canvas
      ref={canvasRef}
      className="fixed inset-0 pointer-events-none z-0"
    />
  )
}