import { Link } from 'react-router-dom'
import type { Mo2Status, AppConfig } from '../types'

interface SystemStatusSectionProps {
  status: Mo2Status | null
  config: AppConfig | null
}

function targetNameFromPath(path?: string): string {
  if (!path) return 'No target'
  const cleaned = path.replace(/[\\/]+$/, '')
  const parts = cleaned.split(/[\\/]/)
  const tail = parts[parts.length - 1] || 'No target'
  return tail
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map(w => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')
}

function MiniStat({ label, value, sub }: { label: string; value: string; sub: string }) {
  return (
    <div>
      <p className="ui-label" style={{ marginBottom: 2, fontSize: 9.5 }}>{label}</p>
      <div className="flex items-baseline" style={{ gap: 4 }}>
        <span
          className="display-serif tabular-nums"
          style={{ fontSize: 17, fontWeight: 400, color: 'var(--ink)', letterSpacing: '-0.02em', lineHeight: 1 }}
        >
          {value}
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 9.5,
            color: 'var(--ink-4)',
            letterSpacing: '0.04em',
          }}
        >
          {sub}
        </span>
      </div>
    </div>
  )
}

export default function SystemStatusSection({
  status,
  config,
}: SystemStatusSectionProps) {
  if (!status || !config) {
    return (
      <div
        style={{
          padding: '14px 18px',
          background: 'var(--card-2)',
          display: 'flex',
          flexDirection: 'column',
          gap: 12,
        }}
      >
        <div>
          <div className="skeleton-line" style={{ height: 8, width: 60, marginBottom: 10 }} />
          <div className="skeleton-line" style={{ height: 18, width: 140, marginBottom: 6 }} />
          <div className="skeleton-line" style={{ height: 10, width: 180 }} />
        </div>
        <div style={{ height: 1, background: 'var(--rule-soft)' }} />
        <div className="grid" style={{ gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          {[0, 1].map(i => (
            <div key={i}>
              <div className="skeleton-line" style={{ height: 8, width: 50, marginBottom: 6 }} />
              <div className="skeleton-line" style={{ height: 18, width: 60 }} />
            </div>
          ))}
        </div>
      </div>
    )
  }

  const targetName = targetNameFromPath(config.mo2ModsPath)
  const showWarning = !status.configured

  return (
    <div
      style={{
        padding: '14px 18px',
        background: 'var(--card-2)',
        display: 'flex',
        flexDirection: 'column',
        gap: 10,
        height: '100%',
      }}
    >
      {/* TARGET */}
      <div>
        <p className="ui-label" style={{ marginBottom: 4, fontSize: 9.5 }}>Target</p>
        <p
          className="display-serif"
          style={{
            fontSize: 15,
            color: 'var(--ink)',
            letterSpacing: '-0.01em',
            lineHeight: 1.15,
            margin: 0,
          }}
          title={config.mo2ModsPath}
        >
          {targetName}
        </p>
        <p
          className="timestamp-print"
          style={{
            marginTop: 2,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
          title={config.mo2ModsPath}
        >
          {config.mo2ModsPath || '<unset>'}
        </p>
      </div>

      <div style={{ height: 1, background: 'var(--rule-soft)' }} />

      {/* MINI STATS - Installed + FOMODs only (Last scan/Session live in sidebar) */}
      <div className="grid" style={{ gridTemplateColumns: '1fr 1fr', gap: '8px 14px' }}>
        <MiniStat label="Installed" value={String(status.modCount)} sub="mods" />
        <MiniStat label="FOMODs"    value={String(status.jsonCount)} sub="parsed" />
      </div>

      {/* Configuration warning */}
      {showWarning && (
        <>
          <div style={{ height: 1, background: 'var(--rule-soft)' }} />
          <div
            style={{
              padding: '10px 12px',
              background: 'var(--paper-3)',
              border: '1px solid var(--rule)',
              borderRadius: 'var(--radius-sm)',
            }}
          >
            <p className="ui-label" style={{ marginBottom: 4, color: 'var(--ochre)' }}>
              Setup needed
            </p>
            <p className="timestamp-print" style={{ fontSize: 11, color: 'var(--ink-3)' }}>
              // configure mo2 paths in{' '}
              <Link
                to="/settings"
                style={{ color: 'var(--accent)', textDecoration: 'underline', textUnderlineOffset: 2 }}
              >
                settings
              </Link>
            </p>
          </div>
        </>
      )}
    </div>
  )
}
