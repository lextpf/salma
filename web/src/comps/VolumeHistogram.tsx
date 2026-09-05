import type { HistogramBucket } from '../logParse'

interface VolumeHistogramProps {
  buckets: HistogramBucket[]
  live: boolean
  onToggleLive?: () => void
  errors?: number
  warnings?: number
  passes?: number
  showPasses?: boolean
}

function StatDot({ color, value, label }: { color: string; value: number; label: string }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 7 }}>
      <span
        aria-hidden="true"
        style={{ width: 6, height: 6, borderRadius: 'var(--radius-full)', background: color }}
      />
      <span className="tabular-nums" style={{ fontSize: 'var(--fs-title)', fontWeight: 600, color }}>
        {value.toLocaleString()}
      </span>
      <span
        style={{
          fontSize: 'var(--fs-micro)',
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-chip)',
          color: 'var(--ink-5)',
        }}
      >
        {label}
      </span>
    </span>
  )
}

export default function VolumeHistogram({
  buckets,
  live,
  onToggleLive,
  errors = 0,
  warnings = 0,
  passes = 0,
  showPasses = false,
}: VolumeHistogramProps) {
  const total = buckets.reduce((sum, b) => sum + b.count, 0)
  const maxCount = Math.max(1, ...buckets.map((b) => b.count))

  return (
    <div
      style={{
        flexShrink: 0,
        padding: '15px 18px 14px',
        background: 'var(--paper)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'flex-end',
          gap: 20,
          marginBottom: 12,
          fontFamily: 'var(--font-mono)',
          flexWrap: 'wrap',
        }}
      >
        <span style={{ display: 'inline-flex', flexDirection: 'column', gap: 3, flexShrink: 0 }}>
          <span
            style={{
              fontSize: 'var(--fs-micro)',
              fontWeight: 600,
              textTransform: 'uppercase',
              letterSpacing: 'var(--tr-kicker)',
              color: 'var(--ink-faint)',
            }}
          >
            Volume
          </span>
          <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 6 }}>
            <span
              className="tabular-nums"
              style={{
                fontSize: 'var(--fs-hero-sm)',
                fontWeight: 600,
                letterSpacing: 'var(--tr-hero)',
                lineHeight: 0.9,
                color: 'var(--ink)',
              }}
            >
              {total.toLocaleString()}
            </span>
            <span style={{ fontFamily: 'var(--font-body)', fontSize: 'var(--fs-mono)', color: 'var(--ink-6)' }}>
              records
            </span>
          </span>
        </span>

        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 16, paddingBottom: 3 }}>
          <StatDot color="var(--danger)" value={errors} label="err" />
          <StatDot color="var(--brass)" value={warnings} label="warn" />
          <StatDot
            color="var(--moss)"
            value={showPasses ? passes : Math.max(0, total - errors - warnings)}
            label={showPasses ? 'pass' : 'ok'}
          />
        </span>

        <span style={{ flex: 1 }} />

        <span
          className="tabular-nums"
          style={{
            flexShrink: 0,
            paddingBottom: 6,
            fontSize: 'var(--fs-micro)',
            letterSpacing: 'var(--tr-chip)',
            textTransform: 'uppercase',
            color: 'var(--ink-5)',
          }}
        >
          {buckets.length} cols &middot; peak {maxCount.toLocaleString()}
        </span>

        <button
          type="button"
          className="btn"
          onClick={onToggleLive}
          aria-label={live ? 'Pause log tailing' : 'Resume log tailing'}
          aria-pressed={live}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 8,
            flexShrink: 0,
            height: 27,
            padding: '0 11px',
            borderRadius: 'var(--radius-ctrl)',
            border: `1px solid ${live ? 'var(--signal-bd)' : 'var(--rule-ctrl)'}`,
            background: live ? 'var(--signal-wash-chip)' : 'var(--btn-bg)',
            cursor: 'pointer',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-meta)',
            color: live ? 'var(--signal-2)' : 'var(--ink-5)',
          }}
        >
          <span
            aria-hidden="true"
            className={live ? 'blink' : undefined}
            style={{
              width: 5,
              height: 5,
              borderRadius: 'var(--radius-full)',
              background: live ? 'var(--signal)' : 'var(--ink-5)',
            }}
          />
          {live ? 'tailing' : 'paused'}
        </button>
      </div>

      <div style={{ position: 'relative', height: 46 }}>
        {[25, 50, 75].map(t => (
          <span
            key={t}
            aria-hidden="true"
            style={{
              position: 'absolute',
              left: 0,
              right: 0,
              bottom: `${t}%`,
              height: 1,
              background: 'var(--rule-faint)',
            }}
          />
        ))}
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'flex-end', gap: 2 }}>
          {buckets.map((b, i) => {
            const hot = b.error > 0
            const warm = !hot && b.warn > 0
            const pct = total === 0 ? 0 : Math.round((b.count / maxCount) * 100)
            const background = hot
              ? 'var(--danger)'
              : warm
                ? 'var(--brass)'
                : 'var(--hist-bar)'
            return (
              <span
                key={i}
                title={`${b.count} records`}
                style={{
                  flex: 1,
                  minWidth: 0,
                  height: `${pct}%`,
                  minHeight: 3,
                  background,
                }}
              />
            )
          })}
        </div>
        <span
          aria-hidden="true"
          style={{
            position: 'absolute',
            left: 0,
            right: 0,
            bottom: 0,
            height: 1,
            background: 'var(--rule-strong)',
          }}
        />
      </div>
    </div>
  )
}
