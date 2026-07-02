import { memo } from 'react'
import { NavLink } from 'react-router-dom'
import MIcon from './MIcon'
import { RAIL_WIDTH, RAIL_WIDTH_ICON } from '../useViewportNarrow'

interface Module {
  to: string
  num: string
  label: string
  icon: string
  end?: boolean
}

const MODULES: Module[] = [
  { to: '/', num: '01', label: 'Install', icon: 'cloud_upload', end: true },
  { to: '/fomods', num: '02', label: 'Library', icon: 'inventory_2' },
  { to: '/logs', num: '03', label: 'Logs', icon: 'receipt_long' },
  { to: '/settings', num: '04', label: 'Settings', icon: 'settings' },
]

interface ModuleRailProps {
  inferredCount: number
  modCount: number
  partialCount: number | null
  modsPath?: string
  collapsed?: boolean
  onToggle?: () => void
}

const sectionLabel: React.CSSProperties = {
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--fs-micro)',
  fontWeight: 600,
  textTransform: 'uppercase',
  letterSpacing: 'var(--tr-kicker)',
  color: 'var(--ink-faint)',
}

function MetaRow({ label, value }: { label: string; value: string }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'baseline',
        justifyContent: 'space-between',
        gap: 8,
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
      }}
    >
      <span
        style={{
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-chip)',
          color: 'var(--ink-5)',
          flexShrink: 0,
        }}
      >
        {label}
      </span>
      <span className="tabular-nums" style={{ color: 'var(--ink-4)' }}>
        {value}
      </span>
    </div>
  )
}

// `Library` stays active on detail routes because its link does not require an exact match.
// the status clock updates once per second. memoization keeps this rail stable.
function ModuleRail({ inferredCount, modCount, partialCount, modsPath, collapsed = false, onToggle }: ModuleRailProps) {
  const resolved = modCount > 0 ? Math.min(100, Math.round((inferredCount / modCount) * 100)) : 0

  const toggle = onToggle && (
    <button
      type="button"
      className="btn"
      onClick={onToggle}
      title={collapsed ? 'Expand the module rail' : 'Collapse the module rail'}
      aria-label={collapsed ? 'Expand the module rail' : 'Collapse the module rail'}
      aria-expanded={!collapsed}
      style={{
        width: 22,
        height: 22,
        flexShrink: 0,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        border: 'none',
        background: 'transparent',
        borderRadius: 'var(--radius-chip)',
        color: 'var(--ink-5)',
        cursor: 'pointer',
      }}
    >
      <MIcon name={collapsed ? 'chevron_right' : 'chevron_left'} size={16} />
    </button>
  )

  return (
    <nav
      style={{
        width: collapsed ? RAIL_WIDTH_ICON : RAIL_WIDTH,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        padding: '18px 0 0',
        transition: 'width 160ms var(--ease)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: collapsed ? '0 0 12px' : '0 10px 12px 14px',
          justifyContent: collapsed ? 'center' : undefined,
        }}
      >
        {!collapsed && <span style={{ ...sectionLabel, flex: 1 }}>Modules</span>}
        {toggle}
      </div>

      {MODULES.map((m) => (
        <NavLink
          key={m.to}
          to={m.to}
          end={m.end}
          className="rail-item"
          title={collapsed ? `${m.num} ${m.label}` : undefined}
          style={({ isActive }) => ({
            display: 'flex',
            alignItems: 'center',
            gap: collapsed ? 0 : 11,
            justifyContent: collapsed ? 'center' : undefined,
            height: 34,
            padding: collapsed ? '0 0 0 2px' : '0 16px 0 12px',
            borderLeft: `2px solid ${isActive ? 'var(--sel-edge)' : 'transparent'}`,
            textDecoration: 'none',
            color: isActive ? 'var(--ink)' : 'var(--ink-4)',
            background: isActive ? 'var(--sel-bg)' : undefined,
          })}
        >
          {({ isActive }) => (
            <>
              {!collapsed && (
                <span
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--fs-micro)',
                    fontWeight: isActive ? 700 : 500,
                    width: 15,
                    color: isActive ? 'var(--signal)' : 'var(--ink-5)',
                  }}
                >
                  {m.num}
                </span>
              )}
              <span style={{ width: 16, display: 'flex', justifyContent: 'center', flexShrink: 0 }}>
                <MIcon name={m.icon} size={17} fill={isActive} />
              </span>
              {!collapsed && (
                <span
                  style={{
                    flex: 1,
                    minWidth: 0,
                    fontSize: 'var(--fs-body)',
                    fontWeight: isActive ? 600 : 500,
                    letterSpacing: '-0.01em',
                  }}
                >
                  {m.label}
                </span>
              )}
              {!collapsed && m.to === '/fomods' && inferredCount > 0 && (
                <span
                  className="tabular-nums"
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--fs-micro)',
                    fontWeight: 600,
                    padding: '1px 7px',
                    borderRadius: 'var(--radius-chip)',
                    border: `1px solid ${isActive ? 'var(--signal-bd)' : 'var(--rule-soft)'}`,
                    background: isActive ? 'var(--signal-wash-chip)' : 'transparent',
                    color: isActive ? 'var(--signal-2)' : 'var(--ink-5)',
                  }}
                >
                  {inferredCount}
                </span>
              )}
            </>
          )}
        </NavLink>
      ))}

      <div aria-hidden="true" style={{ flex: 1, minHeight: 20 }} />

      {collapsed ? (
        <div
          className="tabular-nums"
          title={`${inferredCount} inferred FOMODs of ${modCount} mods`}
          style={{
            flexShrink: 0,
            padding: '12px 0 14px',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            gap: 5,
            fontFamily: 'var(--font-mono)',
          }}
        >
          <span aria-hidden="true" style={{ width: 4, height: 4, background: 'var(--signal)' }} />
          <span style={{ fontSize: 'var(--fs-title)', fontWeight: 600, color: 'var(--ink)' }}>
            {inferredCount}
          </span>
          <span style={{ fontSize: 'var(--fs-micro)', color: 'var(--ink-faint)' }}>{resolved}%</span>
        </div>
      ) : (
      <div
        style={{
          flexShrink: 0,
          padding: '12px 16px 14px 14px',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
          <span aria-hidden="true" style={{ width: 4, height: 4, background: 'var(--signal)' }} />
          <span style={sectionLabel}>Inferred</span>
        </div>

        <div style={{ display: 'flex', alignItems: 'baseline', gap: 7, margin: '2px 0 8px' }}>
          <span
            className="tabular-nums"
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-hero)',
              fontWeight: 600,
              letterSpacing: 'var(--tr-hero)',
              lineHeight: 1,
              color: 'var(--ink)',
            }}
          >
            {inferredCount}
          </span>
          <span style={{ fontSize: 'var(--fs-label)', color: 'var(--ink-5)' }}>FOMODs</span>
        </div>

        <div style={{ height: 3, background: 'var(--track)' }}>
          <div
            style={{
              height: '100%',
              width: `${resolved}%`,
              background: 'var(--signal)',
              transition: 'width 220ms var(--ease)',
            }}
          />
        </div>

        <div
          style={{
            display: 'flex',
            alignItems: 'baseline',
            justifyContent: 'space-between',
            gap: 8,
            marginTop: 6,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
          }}
        >
          <span className="tabular-nums" style={{ color: 'var(--ink-5)' }}>
            {resolved}% resolved
          </span>
          {partialCount != null && partialCount > 0 && (
            <span className="tabular-nums" style={{ color: 'var(--brass)' }}>
              {partialCount} partial
            </span>
          )}
        </div>

        <div style={{ marginTop: 7 }}>
          <MetaRow label="Mods" value={String(modCount)} />
        </div>

        <div
          title={modsPath}
          style={{
            marginTop: 3,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            color: 'var(--ink-5)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {modsPath || 'no mods path set'}
        </div>
      </div>
      )}
    </nav>
  )
}

export default memo(ModuleRail)
