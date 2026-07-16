import type { HistogramBucket } from '../../logParse'

interface VolumeHistogramProps {
  buckets: HistogramBucket[]
  // Reflects the live/paused tail state; drives the blinking LED.
  live: boolean
  onToggleLive?: () => void
  errors?: number
  warnings?: number
  passes?: number
  // When true (test.log) the moss "pass" count is shown alongside err/warn.
  showPasses?: boolean
}

function StatDot({ color, value, label }: { color: string; value: number; label: string }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
      <span style={{ width: 5, height: 5, borderRadius: '50%', background: color }} />
      <span style={{ color: 'var(--ink-3)' }}>{value}</span>
      <span style={{ color: 'var(--ink-5)', letterSpacing: '0.04em' }}>{label}</span>
    </span>
  )
}

// The recent-activity strip: a "Volume" header with err/warn counts, a blinking
// "tailing" LED toggle, and 12 bars whose color escalates to ochre/danger when a
// bucket carries a WARN/ERROR.
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
        padding: '13px 18px',
        borderBottom: '1px solid var(--rule-soft)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: 9,
        }}
      >
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 10,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            letterSpacing: '0.12em',
            textTransform: 'uppercase',
            color: 'var(--ink-5)',
          }}
        >
          <span>
            Volume <span style={{ color: 'var(--ink-6)' }}>&middot;</span> {total} records
          </span>
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 9,
              textTransform: 'none',
              letterSpacing: 0,
              fontSize: 'var(--fs-micro)',
            }}
          >
            <StatDot color="var(--danger)" value={errors} label="err" />
            <StatDot color="var(--ochre)" value={warnings} label="warn" />
            {showPasses && <StatDot color="var(--moss)" value={passes} label="pass" />}
          </span>
        </span>

        <button
          type="button"
          onClick={onToggleLive}
          aria-label={live ? 'Pause log tailing' : 'Resume log tailing'}
          aria-pressed={live}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 7,
            background: 'transparent',
            border: 'none',
            padding: 0,
            cursor: 'pointer',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            color: 'var(--ink-4)',
          }}
        >
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              background: live ? 'var(--ink)' : 'var(--ink-faint)',
              animation: live ? 'salma-blink 1.8s infinite' : 'none',
            }}
          />
          {live ? 'tailing' : 'paused'}
        </button>
      </div>

      <div style={{ display: 'flex', alignItems: 'flex-end', gap: 3, height: 36 }}>
        {buckets.map((b, i) => {
          const hot = b.error > 0
          const warm = !hot && b.warn > 0
          const color = hot ? 'var(--danger)' : warm ? 'var(--ochre)' : 'var(--ink)'
          const pct = total === 0 ? 0 : Math.round((b.count / maxCount) * 100)
          return (
            <span
              key={i}
              title={`${b.count} records`}
              style={{
                flex: 1,
                height: `${pct}%`,
                minHeight: 3,
                borderRadius: 1,
                background: color,
                opacity: hot || warm ? 1 : 0.5,
              }}
            />
          )
        })}
      </div>
    </div>
  )
}
