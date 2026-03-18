import { NavLink } from 'react-router-dom'

interface Module {
  to: string
  num: string
  label: string
  icon: string
  end?: boolean
}

const MODULES: Module[] = [
  { to: '/', num: '01', label: 'Install', icon: 'fa-cloud-arrow-up', end: true },
  { to: '/fomods', num: '02', label: 'Library', icon: 'fa-box-archive' },
  { to: '/logs', num: '03', label: 'Logs', icon: 'fa-scroll' },
  { to: '/settings', num: '04', label: 'Settings', icon: 'fa-gear' },
]

interface ModuleRailProps {
  inferredCount: number
  modsPath?: string
}

// The 192px left rail: numbered module links (Library stays active on its detail
// route via end=false), plus a footer tally of inferred FOMODs and the mods path.
export default function ModuleRail({ inferredCount, modsPath }: ModuleRailProps) {
  return (
    <nav
      style={{
        width: 192,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--paper)',
        borderRight: '1px solid var(--rule-soft)',
        padding: '18px 0',
      }}
    >
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          textTransform: 'uppercase',
          letterSpacing: '0.2em',
          color: 'var(--ink-5)',
          padding: '0 18px 12px',
        }}
      >
        Modules
      </div>

      {MODULES.map((m) => (
        <NavLink
          key={m.to}
          to={m.to}
          end={m.end}
          className="fm-rail-item"
          style={({ isActive }) => ({
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '8px 16px 8px 12px',
            textDecoration: 'none',
            borderLeft: `2.5px solid ${isActive ? 'var(--ink)' : 'transparent'}`,
            background: isActive ? 'var(--sheet)' : undefined,
            boxShadow: isActive ? 'inset 0 0 0 1px var(--rule-soft)' : 'none',
            color: isActive ? 'var(--ink)' : 'var(--ink-3)',
          })}
        >
          {({ isActive }) => (
            <>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  width: 15,
                  color: isActive ? 'var(--ink)' : 'var(--ink-5)',
                }}
              >
                {m.num}
              </span>
              <i
                className={`fa-duotone fa-solid ${m.icon}`}
                style={{ fontSize: 'var(--fs-title)', width: 16, textAlign: 'center' }}
              />
              <span style={{ flex: 1, fontSize: 'var(--fs-title)', fontWeight: 500 }}>{m.label}</span>
              {m.to === '/fomods' && inferredCount > 0 && (
                <span
                  className="tabular-nums"
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--fs-micro)',
                    padding: '1px 6px',
                    borderRadius: 999,
                    background: isActive ? 'var(--ink)' : 'var(--paper-3)',
                    color: isActive ? 'var(--sheet)' : 'var(--ink-4)',
                  }}
                >
                  {inferredCount}
                </span>
              )}
            </>
          )}
        </NavLink>
      ))}

      <div style={{ flex: 1 }} />

      <div style={{ padding: '14px 18px 0', borderTop: '1px solid var(--rule-soft)', margin: '0 0' }}>
        <div
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            textTransform: 'uppercase',
            letterSpacing: '0.2em',
            color: 'var(--ink-5)',
            marginBottom: 6,
          }}
        >
          Inferred
        </div>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 6 }}>
          <span
            className="tabular-nums"
            style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-display)', fontWeight: 600, color: 'var(--ink)' }}
          >
            {inferredCount}
          </span>
          <span style={{ fontSize: 'var(--fs-label)', color: 'var(--ink-4)' }}>FOMODs</span>
        </div>
        <div
          title={modsPath}
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            color: 'var(--ink-5)',
            marginTop: 4,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {modsPath || 'no mods path set'}
        </div>
      </div>
    </nav>
  )
}
