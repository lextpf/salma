import { useState, useEffect, useRef } from 'react'
import { NavLink, Outlet } from 'react-router-dom'
import { useTheme } from '../ThemeContext'
import { getMo2Status } from '../api'

const navItems = [
  { to: '/', label: 'Install', icon: 'fa-duotone fa-solid fa-cloud-arrow-up', grad: 'icon-gradient-nebula' },
  { to: '/fomods', label: 'FOMODs', icon: 'fa-duotone fa-solid fa-box-archive', grad: 'icon-gradient-spring' },
  { to: '/logs', label: 'Logs', icon: 'fa-duotone fa-solid fa-terminal', grad: 'icon-gradient-ember' },
  { to: '/settings', label: 'Settings', icon: 'fa-duotone fa-solid fa-gear', grad: 'icon-gradient-steel' },
]

export default function Layout() {
  const { theme, toggleTheme } = useTheme()
  const [backendUp, setBackendUp] = useState<boolean | null>(null)
  const inFlightRef = useRef(false)

  useEffect(() => {
    const check = () => {
      if (inFlightRef.current) return
      inFlightRef.current = true
      getMo2Status()
        .then(() => setBackendUp(true))
        .catch(() => setBackendUp(false))
        .finally(() => { inFlightRef.current = false })
    }
    check()
    const id = setInterval(check, 8000)
    return () => clearInterval(id)
  }, [])

  return (
    <div className="flex h-screen overflow-hidden">
      {/* Sidebar */}
      <nav className="relative w-sidebar shrink-0 bg-gradient-to-b from-surface-container via-surface-container to-surface-dim border-r border-outline-variant/40 flex flex-col">
        {/* Aurora accent line on right edge */}
        <div className="sidebar-aurora-line" />

        {/* Brand */}
        <div className="px-5 pt-7 pb-5 flex items-start justify-between">
          <div>
            <h1 className="text-[1.7rem] font-extrabold text-gradient leading-tight tracking-tight flex items-center gap-2.5">
              <i className="fa-duotone fa-solid fa-foot-wing icon-gradient icon-gradient-aurora text-xl" />
              salma
            </h1>
            <p className="text-[0.6rem] uppercase tracking-[0.2em] text-outline font-semibold mt-0.5">mod installer</p>
          </div>
          <button
            onClick={toggleTheme}
            className="mt-1 p-1.5 rounded-lg text-on-surface-variant hover:text-primary transition-colors duration-150"
            aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`}
          >
            <i className={`fa-duotone fa-solid ${theme === 'dark' ? 'fa-sun' : 'fa-moon'} text-sm`} />
          </button>
        </div>

        {/* Divider */}
        <div className="mx-4 mb-3 aurora-divider" />

        {/* Navigation */}
        <ul className="flex-1 px-3 space-y-0.5">
          {navItems.map(item => (
            <li key={item.to}>
              <NavLink
                to={item.to}
                end={item.to === '/'}
                className={({ isActive }) =>
                  `flex items-center gap-3 px-3.5 py-2.5 rounded-xl text-[0.825rem] font-medium transition-all duration-150 border
                   ${isActive
                     ? 'nav-active text-primary-light border-transparent'
                     : 'text-on-surface-variant border-transparent hover:bg-surface-container-high hover:text-on-surface hover:border-outline-variant/35'
                   }`
                }
              >
                <span className="w-5 text-center text-[0.9rem]">
                  <i className={`${item.icon} icon-gradient ${item.grad}`} />
                </span>
                <span>{item.label}</span>
              </NavLink>
            </li>
          ))}
        </ul>

        {/* Footer */}
        <div className="px-4 py-4">
          <div className="aurora-divider mb-3" />
          <div className="flex items-center gap-2 text-[0.625rem] text-outline">
            {backendUp === null ? (
              <>
                <span className="w-2 h-2 rounded-full bg-outline/40" />
                <span className="text-outline font-medium">Checking...</span>
              </>
            ) : backendUp ? (
              <>
                <span
                  className="w-2 h-2 rounded-full shadow-[0_0_6px_2px_rgba(34,197,94,0.45)]"
                  style={{ background: 'conic-gradient(rgba(34,197,94,0.9) 0deg, rgba(52,211,153,0.85) 180deg, rgba(34,197,94,0.9) 360deg)' }}
                />
                <span className="text-success font-medium">Connected</span>
              </>
            ) : (
              <>
                <span className="w-2 h-2 rounded-full bg-error shadow-[0_0_6px_2px_rgba(239,68,68,0.45)]" />
                <span className="text-error font-medium">Disconnected</span>
              </>
            )}
            <span className="text-outline/60">-</span>
            <span>v1.0.0</span>
          </div>
        </div>
      </nav>

      {/* Main content - static aurora gradient bg, no animations */}
      <main className="flex-1 overflow-y-auto aurora-bg">
        <div className="content-shell mx-auto px-8 py-8">
          <Outlet />
        </div>
      </main>
    </div>
  )
}

