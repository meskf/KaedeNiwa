import { useState, useEffect } from 'react'
import { type Theme, getStoredTheme, applyTheme } from './theme'

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(getStoredTheme)
  useEffect(() => { applyTheme(theme) }, [theme])
  return { theme, setTheme }
}
