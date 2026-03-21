import MIcon from '../../MIcon'
import Chip from '../../Chip'
import { normalizeGroups, type NormalizedGroup, type NormalizedPlugin } from '../../../fomodNormalize'
import type { FomodDetail } from '../../../types'

interface StepsTabProps {
  detail: FomodDetail
}

function OptionRow({ option }: { option: NormalizedPlugin }) {
  const sel = option.selected
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        padding: '8px 11px',
        borderRadius: 7,
        background: sel ? 'var(--card-2)' : 'transparent',
        border: `1px solid ${sel ? 'var(--rule)' : 'transparent'}`,
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 16,
          height: 16,
          borderRadius: 5,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexShrink: 0,
          background: sel ? 'var(--ink)' : 'transparent',
          border: `1px solid ${sel ? 'var(--ink)' : 'var(--rule-strong)'}`,
        }}
      >
        {sel && <MIcon name="check" size={12} weight={600} style={{ color: 'var(--sheet)' }} />}
      </span>
      <span style={{ fontSize: 'var(--fs-body)', color: sel ? 'var(--ink)' : 'var(--ink-4)' }}>{option.name}</span>
      {sel && (
        <span
          style={{
            marginLeft: 'auto',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            letterSpacing: '0.08em',
            textTransform: 'uppercase',
            color: 'var(--ink)',
          }}
        >
          selected
        </span>
      )}
    </div>
  )
}

function GroupBlock({ group, index }: { group: NormalizedGroup; index: number }) {
  const options = [...group.plugins, ...group.deselected]
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 9, flexWrap: 'wrap' }}>
        <span style={{ fontSize: 'var(--fs-label)', fontWeight: 600, color: 'var(--ink-2)' }}>
          {group.name || `Group ${index + 1}`}
        </span>
        {group.resolved_by && <Chip label={group.resolved_by} color="var(--ink-4)" title="Resolution source" />}
      </div>
      {options.length === 0 ? (
        <p
          style={{
            margin: 0,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            color: 'var(--ink-6)',
          }}
        >
          // no options recorded
        </p>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
          {options.map((option, oi) => (
            <OptionRow key={`${option.name}-${oi}`} option={option} />
          ))}
        </div>
      )}
    </div>
  )
}

// Steps tab: one card per inference step, each listing its groups and option
// rows (filled-ink checkbox when the option was selected). Reuses the shared
// schema-v2 normalization so it tolerates legacy plugin/group shapes.
export default function StepsTab({ detail }: StepsTabProps) {
  const steps = detail.steps ?? []

  if (steps.length === 0) {
    return (
      <p style={{ margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-label)', color: 'var(--ink-5)' }}>
        // no steps in this record
      </p>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {steps.map((step, si) => {
        const groups = normalizeGroups(step)
        return (
          <div
            key={step.name || `step-${si}`}
            style={{ border: '1px solid var(--rule-soft)', borderRadius: 9, overflow: 'hidden' }}
          >
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 11,
                padding: '9px 14px',
                background: 'var(--card)',
                borderBottom: '1px solid var(--rule-soft)',
              }}
            >
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  letterSpacing: '0.08em',
                  color: 'var(--ink-4)',
                }}
              >
                STEP {String(si + 1).padStart(2, '0')}
              </span>
              <span aria-hidden="true" style={{ width: 14, height: 1, background: 'var(--rule-strong)' }} />
              <span style={{ fontSize: 'var(--fs-body)', fontWeight: 600, color: 'var(--ink)' }}>
                {step.name || `Step ${si + 1}`}
              </span>
            </div>
            <div style={{ padding: '13px 14px', display: 'flex', flexDirection: 'column', gap: 14 }}>
              {groups.length === 0 ? (
                <p style={{ margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-6)' }}>
                  // no groups in this step
                </p>
              ) : (
                groups.map((group, gi) => <GroupBlock key={`${group.name}-${gi}`} group={group} index={gi} />)
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}
