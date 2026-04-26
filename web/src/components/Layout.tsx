import { useState, useEffect, useRef } from 'react'
import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { useTheme } from '../ThemeContext'
import { getMo2Status, getFomodScanStatus, getTestStatus, getInstallStatus } from '../api'
import type { Mo2Status } from '../types'

type EngineState = 'idle' | 'scanning' | 'testing' | 'installing'

const engineLabel: Record<EngineState, { value: string; color: string; pulse: boolean }> = {
  idle:       { value: 'ready',   color: 'var(--moss)',     pulse: false },
  scanning:   { value: 'scan',    color: 'var(--ochre)',    pulse: true  },
  testing:    { value: 'test',    color: 'var(--ink-blue)', pulse: true  },
  installing: { value: 'install', color: 'var(--accent)',   pulse: true  },
}

interface NavItem {
  to: string
  label: string
  num: string
  icon: string
  end?: boolean
}

const navItems: NavItem[] = [
  { to: '/',         label: 'Install',  num: '01', icon: 'fa-duotone fa-solid fa-cloud-arrow-up', end: true },
  { to: '/fomods',   label: 'FOMODs',   num: '02', icon: 'fa-duotone fa-solid fa-box-archive' },
  { to: '/logs',     label: 'Logs',     num: '03', icon: 'fa-duotone fa-solid fa-scroll' },
  { to: '/settings', label: 'Settings', num: '04', icon: 'fa-duotone fa-solid fa-gear' },
]

const breadcrumbForPath: Record<string, { chapter: string; section: string; title: string }> = {
  '/':         { chapter: '01', section: 'Atelier',   title: 'Install / Upload' },
  '/fomods':   { chapter: '02', section: 'Library',   title: 'FOMODs / Browse' },
  '/logs':     { chapter: '03', section: 'Stream',    title: 'Logs / Tail' },
  '/settings': { chapter: '04', section: 'Atelier',   title: 'Settings / Config' },
}

function pad(n: number) {
  return String(n).padStart(2, '0')
}

function formatClock(d: Date) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function formatElapsed(ms: number) {
  const sec = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(sec / 3600)
  const m = Math.floor((sec % 3600) / 60)
  const s = sec % 60
  if (h > 0) return `${h}h ${pad(m)}m`
  if (m > 0) return `${m}m ${pad(s)}s`
  return `${s}s`
}

export default function Layout() {
  const { theme, toggleTheme } = useTheme()
  const location = useLocation()

  const [status, setStatus] = useState<Mo2Status | null>(null)
  const [backendUp, setBackendUp] = useState<boolean | null>(null)
  const [scanRunning, setScanRunning] = useState(false)
  const [testRunning, setTestRunning] = useState(false)
  const [installRunning, setInstallRunning] = useState(false)
  const [now, setNow] = useState(() => new Date())
  const [sessionStartMs] = useState(() => Date.now())
  const inFlightRef = useRef(false)

  useEffect(() => {
    const check = () => {
      if (inFlightRef.current) return
      inFlightRef.current = true
      Promise.all([
        getMo2Status().catch(() => null),
        getFomodScanStatus().catch(() => null),
        getTestStatus().catch(() => null),
        getInstallStatus().catch(() => null),
      ])
        .then(([s, scan, test, install]) => {
          if (s) { setStatus(s); setBackendUp(true) }
          else { setBackendUp(false) }
          setScanRunning(scan?.running === true)
          setTestRunning(test?.running === true)
          setInstallRunning(install?.running === true)
        })
        .finally(() => { inFlightRef.current = false })
    }
    check()
    const id = setInterval(check, 8000)
    return () => clearInterval(id)
  }, [])

  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000)
    return () => clearInterval(id)
  }, [])

  const mo2On      = backendUp === true && status?.configured === true && status?.pluginInstalled === true
  const serverOn   = backendUp === true
  const dllLoaded  = status?.pluginInstalled === true

  const engineState: EngineState =
    installRunning ? 'installing' :
    scanRunning    ? 'scanning'   :
    testRunning    ? 'testing'    :
    'idle'
  const engine = engineLabel[engineState]

  const breadcrumb =
    breadcrumbForPath[location.pathname] ||
    (location.pathname.startsWith('/fomods/')
      ? { chapter: '02', section: 'Library', title: 'FOMODs / Detail' }
      : { chapter: '01', section: 'Atelier', title: 'Install' })

  return (
    <div className="flex h-screen overflow-hidden">
      {/* ============================================================
          SIDEBAR
          ============================================================ */}
      <aside
        className="shrink-0 sticky top-0 h-screen flex flex-col"
        style={{
          width: 236,
          background: 'var(--paper-2)',
          borderRight: '1px solid var(--rule)',
          padding: '28px 20px 24px',
        }}
      >
        {/* Wordmark */}
        <div style={{ marginBottom: 36 }}>
          <div className="flex items-baseline" style={{ gap: 6 }}>
            <span
              className="display-serif-italic"
              style={{ fontSize: 30, color: 'var(--ink)', lineHeight: 1 }}
            >
              salma
            </span>
            <span
              aria-hidden="true"
              style={{
                width: 5,
                height: 5,
                borderRadius: '50%',
                background: 'var(--accent)',
                marginBottom: 4,
              }}
            />
          </div>
          <div
            className="flex justify-between"
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 9.5,
              letterSpacing: '0.22em',
              textTransform: 'uppercase',
              color: 'var(--ink-4)',
              marginTop: 8,
            }}
          >
            <span>mod installer</span>
            <span style={{ color: 'var(--ink-3)' }}>v1.1.0</span>
          </div>
        </div>

        {/* Hairline divider with oxblood dot at left */}
        <div
          aria-hidden="true"
          style={{
            position: 'relative',
            height: 1,
            background: 'var(--rule)',
            marginBottom: 28,
          }}
        >
          <span
            style={{
              position: 'absolute',
              left: 0,
              top: -2,
              width: 5,
              height: 5,
              background: 'var(--accent)',
              borderRadius: '50%',
            }}
          />
        </div>

        {/* Nav */}
        <nav className="flex flex-col" style={{ gap: 2 }}>
          {navItems.map(item => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.end}
              className={({ isActive }) =>
                `nav-rail-item${isActive ? ' nav-rail-item-active' : ''}`
              }
            >
              <span className="nav-rail-num">{item.num}</span>
              <span className="nav-rail-icon">
                <i className={item.icon} style={{ fontSize: 15 }} />
              </span>
              <span className="nav-rail-label">{item.label}</span>
              {item.to === '/fomods' && status && status.jsonCount > 0 ? (
                <span className="nav-rail-count tabular-nums">{status.jsonCount}</span>
              ) : null}
            </NavLink>
          ))}
        </nav>

        {/* Flex spacer */}
        <div style={{ flex: 1 }} />

        {/* System status block */}
        <div
          style={{
            padding: '14px 0',
            borderTop: '1px solid var(--rule-soft)',
            marginBottom: 12,
          }}
        >
          <div className="ui-label" style={{ marginBottom: 10, fontSize: 9.5 }}>
            System
          </div>
          <StatRow label="mo2"    value={mo2On     ? 'connected' : (backendUp === false ? 'offline' : 'checking')} on={mo2On} />
          <StatRow label="server" value={serverOn  ? ':5000'     : 'down'}                                          on={serverOn} />
          <StatRow label="dll"    value={dllLoaded ? 'loaded'    : 'missing'}                                       on={dllLoaded} />
          <StatRow label="session" value={formatElapsed(now.getTime() - sessionStartMs)}                            on neutral />
          <StatRow label="engine"  value={engine.value} on dotColor={engine.color} pulse={engine.pulse} />
        </div>

        {/* Theme toggle */}
        <button
          type="button"
          onClick={toggleTheme}
          aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`}
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: '10px 12px',
            border: '1px solid var(--rule)',
            borderRadius: 'var(--radius-md)',
            background: 'var(--card-2)',
            transition: 'border-color 150ms ease, background-color 150ms ease',
            cursor: 'pointer',
          }}
          onMouseEnter={e => { e.currentTarget.style.borderColor = 'var(--ink-4)' }}
          onMouseLeave={e => { e.currentTarget.style.borderColor = 'var(--rule)' }}
        >
          <span
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 10.5,
              letterSpacing: '0.14em',
              textTransform: 'uppercase',
              color: 'var(--ink-3)',
            }}
          >
            {theme === 'dark' ? 'Dark' : 'Light'}
          </span>
          <i
            className={`fa-duotone fa-solid ${theme === 'dark' ? 'fa-moon' : 'fa-sun'}`}
            style={{ fontSize: 14, color: 'var(--ink-3)' }}
          />
        </button>
      </aside>

      {/* ============================================================
          MAIN - breadcrumb + page outlet
          ============================================================ */}
      <main className="flex-1 overflow-hidden" style={{ background: 'var(--paper)' }}>
        <div
          className="content-shell mx-auto"
          style={{ maxWidth: 1180, width: '100%', padding: '28px 44px 48px' }}
        >
          <TopBar breadcrumb={breadcrumb} clock={formatClock(now)} />
          <Outlet />
        </div>
      </main>
    </div>
  )
}

/* ================================================================
   Sidebar StatRow
   ================================================================ */
function StatRow({
  label,
  value,
  on,
  neutral,
  dotColor,
  pulse,
}: {
  label: string
  value: string
  on: boolean
  neutral?: boolean
  dotColor?: string
  pulse?: boolean
}) {
  const baseClass = neutral
    ? 'dot-status'
    : on
      ? 'dot-status dot-status-on'
      : 'dot-status dot-status-off'
  const customDot = dotColor
    ? {
        background: dotColor,
        boxShadow: `0 0 0 3px ${
          dotColor === 'var(--moss)'     ? 'rgba(61, 122, 61, 0.12)' :
          dotColor === 'var(--ochre)'    ? 'rgba(166, 122, 42, 0.12)' :
          dotColor === 'var(--ink-blue)' ? 'rgba(42, 58, 90, 0.14)' :
          dotColor === 'var(--accent)'   ? 'rgba(138, 42, 31, 0.14)' :
          'transparent'
        }`,
      }
    : undefined
  return (
    <div className="stat-row">
      <span style={{ letterSpacing: '0.04em' }}>{label}</span>
      <span className="stat-row-value">
        <span>{value}</span>
        <span
          className={`${baseClass}${pulse ? ' dot-status-pulse' : ''}`}
          style={customDot}
        />
      </span>
    </div>
  )
}

/* ================================================================
   TopBar - breadcrumb + clock + Docs button
   ================================================================ */
function TopBar({
  breadcrumb,
  clock,
}: {
  breadcrumb: { chapter: string; section: string; title: string }
  clock: string
}) {
  return (
    <div
      className="flex items-center justify-between"
      style={{ marginBottom: 18, paddingBottom: 8 }}
    >
      <div
        className="flex items-center"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          letterSpacing: '0.2em',
          textTransform: 'uppercase',
          color: 'var(--ink-4)',
          gap: 14,
        }}
      >
        <span style={{ color: 'var(--ink-3)' }}>
          <span style={{ color: 'var(--accent)', fontStyle: 'italic' }}>{breadcrumb.chapter}</span>
          {' / '}
          {breadcrumb.section}
        </span>
        <span style={{ color: 'var(--ink-5)' }}>-</span>
        <span>{breadcrumb.title}</span>
      </div>
      <div className="flex items-center" style={{ gap: 18 }}>
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 10.5, color: 'var(--ink-4)' }}>
          <span style={{ color: 'var(--ink-5)' }}>t.</span> {clock}
        </span>
        <span aria-hidden="true" style={{ width: 1, height: 14, background: 'var(--rule)' }} />
        <a
          href="https://github.com/lextpf/salma"
          target="_blank"
          rel="noreferrer"
          className="tool-btn"
          style={{ padding: '6px 10px', fontSize: 10 }}
        >
          <i className="fa-duotone fa-solid fa-book-open-cover" style={{ fontSize: 12 }} />
          <span
            style={{
              fontFamily: 'var(--font-mono)',
              letterSpacing: '0.12em',
              textTransform: 'uppercase',
              color: 'var(--ink-2)',
            }}
          >
            Docs
          </span>
        </a>
      </div>
    </div>
  )
}
