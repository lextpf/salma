import { createContext, useContext } from 'react'

export type Theme = 'dark' | 'light'
/** What the user chose. 'system' follows the OS and keeps following it. */
export type ThemeMode = 'system' | 'dark' | 'light'

export const ThemeContext = createContext<{
  theme: Theme
  mode: ThemeMode
  setMode: (m: ThemeMode) => void
  toggleTheme: () => void
}>({
  theme: 'light',
  mode: 'system',
  setMode: () => {},
  toggleTheme: () => {},
})

export const useTheme = () => useContext(ThemeContext)
