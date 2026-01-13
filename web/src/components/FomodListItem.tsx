import { Link } from 'react-router-dom'
import type { FomodEntry } from '../types'

interface FomodListItemProps {
  fomod: FomodEntry
  onDelete: (name: string) => void
  disabled?: boolean
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} b`
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} kb`
  return `${(bytes / 1048576).toFixed(1)} mb`
}

function formatDate(epochMs: number): string {
  return new Date(epochMs).toLocaleDateString(undefined, {
    year: 'numeric', month: 'short', day: '2-digit',
  })
}

export default function FomodListItem({ fomod, onDelete, disabled }: FomodListItemProps) {
  return (
    <div
      className="group fomod-list-item"
      style={{
        display: 'grid',
        gridTemplateColumns: '32px 1fr 110px 90px 100px 28px 24px',
        alignItems: 'center',
        gap: 16,
        padding: '14px 28px',
        borderBottom: '1px solid var(--rule-soft)',
        transition: 'background-color 150ms ease',
      }}
      onMouseEnter={e => { e.currentTarget.style.background = 'var(--card-2)' }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
    >
      {/* Icon */}
      <div
        style={{
          width: 28,
          height: 28,
          borderRadius: 'var(--radius-sm)',
          background: 'var(--paper-2)',
          border: '1px solid var(--rule-soft)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--ink-3)',
        }}
      >
        <i className="fa-duotone fa-solid fa-folder" style={{ fontSize: 14 }} />
      </div>

      {/* Name */}
      <Link
        to={`/fomods/${encodeURIComponent(fomod.name)}`}
        style={{
          fontSize: 15.5,
          color: 'var(--ink)',
          textDecoration: 'none',
          letterSpacing: '-0.005em',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {fomod.name}
      </Link>

      {/* Step count */}
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 12,
          letterSpacing: '0.06em',
          color: 'var(--ink-3)',
        }}
      >
        {fomod.stepCount} {fomod.stepCount === 1 ? 'step' : 'steps'}
      </div>

      {/* Size */}
      <div
        className="tabular-nums"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 12,
          color: 'var(--ink-3)',
          letterSpacing: '0.04em',
        }}
      >
        {formatSize(fomod.size)}
      </div>

      {/* Modified date */}
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 12,
          color: 'var(--ink-4)',
          letterSpacing: '0.04em',
          whiteSpace: 'nowrap',
        }}
      >
        {formatDate(fomod.modified)}
      </div>

      {/* Delete (visible on hover) */}
      <button
        type="button"
        className="opacity-0 group-hover:opacity-100"
        onClick={() => onDelete(fomod.name)}
        disabled={disabled}
        title="Delete"
        style={{
          width: 24,
          height: 24,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: 'transparent',
          border: '1px solid transparent',
          borderRadius: 'var(--radius-sm)',
          color: 'var(--ink-4)',
          transition: 'opacity 150ms ease, color 150ms ease, border-color 150ms ease',
          cursor: disabled ? 'not-allowed' : 'pointer',
        }}
        onMouseEnter={e => {
          if (!disabled) {
            e.currentTarget.style.color = 'var(--danger)'
            e.currentTarget.style.borderColor = 'var(--rule)'
          }
        }}
        onMouseLeave={e => {
          e.currentTarget.style.color = 'var(--ink-4)'
          e.currentTarget.style.borderColor = 'transparent'
        }}
      >
        <i className="fa-duotone fa-solid fa-trash-can" style={{ fontSize: 13 }} />
      </button>

      {/* Chevron */}
      <Link
        to={`/fomods/${encodeURIComponent(fomod.name)}`}
        title="View"
        style={{
          width: 24,
          height: 24,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--ink-4)',
          textDecoration: 'none',
        }}
      >
        <i className="fa-duotone fa-solid fa-angle-right" style={{ fontSize: 14 }} />
      </Link>
    </div>
  )
}
