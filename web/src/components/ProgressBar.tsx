interface ProgressBarProps {
  progress: number
  status?: string
  /** Color override for the fill (default: oxblood accent) */
  color?: string
  /** When true, overlays a sweeping shimmer to indicate active work */
  active?: boolean
}

export default function ProgressBar({ progress, status, color, active = true }: ProgressBarProps) {
  const clamped = Math.min(100, Math.max(0, progress))
  return (
    <div className="w-full mt-3">
      <div className="atelier-progress">
        <div
          className={`atelier-progress-fill ${active ? 'atelier-progress-sweep' : ''}`}
          style={{
            width: `${clamped}%`,
            background: color || 'var(--accent)',
          }}
        />
      </div>
      <div
        className="flex justify-between mt-1.5"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 12,
          color: 'var(--ink-3)',
          letterSpacing: '0.04em',
        }}
      >
        {status && <span className="flex-1 truncate">{status}</span>}
        <span className="tabular-nums" style={{ minWidth: 40, textAlign: 'right' }}>
          {Math.round(clamped)}%
        </span>
      </div>
    </div>
  )
}
