import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { ThemeContext, type Theme, type ThemeMode } from './theme'

const KEY = 'salma-theme'

function storedMode(): ThemeMode {
  const v = localStorage.getItem(KEY)
  return v === 'dark' || v === 'light' || v === 'system' ? v : 'system'
}

function systemTheme(): Theme {
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

/**
 * Theme state.
 *
 * Three modes rather than two, because "follow the OS" is a real preference and
 * a plain toggle cannot express it once it has been clicked. With no stored
 * choice the mode is `system`, not light: salma sits beside Mod Organizer 2,
 * most MO2 setups are dark, and a light flash on first run is the wrong first
 * impression.
 */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setMode] = useState<ThemeMode>(storedMode)
  const [system, setSystem] = useState<Theme>(systemTheme)

  useEffect(() => {
    const mq = window.matchMedia?.('(prefers-color-scheme: dark)')
    if (!mq) return
    const onChange = (e: MediaQueryListEvent) => setSystem(e.matches ? 'dark' : 'light')
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])

  const theme: Theme = mode === 'system' ? system : mode

  useEffect(() => {
    document.documentElement.dataset.theme = theme
    localStorage.setItem(KEY, mode)
  }, [theme, mode])

  // The top-bar button is a two-way switch: it always lands on an explicit
  // choice, which is what a user pressing it expects.
  const toggleTheme = useCallback(() => {
    setMode(theme === 'dark' ? 'light' : 'dark')
  }, [theme])

  const value = useMemo(
    () => ({ theme, mode, setMode, toggleTheme }),
    [theme, mode, toggleTheme],
  )

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
}
