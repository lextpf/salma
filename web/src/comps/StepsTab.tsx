import { useState } from 'react'
import Chip from './Chip'
import MIcon from './MIcon'
import ConfidencePill from './ConfidencePill'
import { tierFor } from '../confidence'
import { normalizeGroups, type NormalizedGroup } from '../fomodNormalize'
import type { FomodDetail } from '../types'

interface StepsTabProps {
  detail: FomodDetail
}

function StepBand({ label, meta }: { label: string; meta: string }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 12,
        padding: '5px 10px',
      }}
    >
      <span
        title={label}
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-chip)',
          color: 'var(--ink-5)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {label}
      </span>
      <span
        className="tabular-nums"
        style={{
          flexShrink: 0,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: 'var(--tr-chip)',
          textTransform: 'uppercase',
          color: 'var(--ink-6)',
        }}
      >
        {meta}
      </span>
    </div>
  )
}

function Mark({ on }: { on: boolean }) {
  return (
    <span
      aria-hidden="true"
      style={{
        width: 4,
        height: 4,
        flexShrink: 0,
        background: on ? 'var(--signal)' : 'transparent',
        border: on ? 'none' : '1px solid var(--rule-ctrl)',
      }}
    />
  )
}

// include selection state in the title because the marker is decorative.
function OptionLine({ name, on }: { name: string; on: boolean }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
      <Mark on={on} />
      <span
        title={on ? `${name} (selected)` : `${name} (not selected)`}
        style={{
          fontSize: 'var(--fs-body)',
          fontWeight: on ? 600 : 400,
          color: on ? 'var(--ink-2)' : 'var(--ink-5)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {name}
      </span>
    </div>
  )
}

function GroupRow({ group, index }: { group: NormalizedGroup; index: number }) {
  const [open, setOpen] = useState(false)
  const chosen = group.plugins.filter(p => p.selected)
  const others = [...group.plugins.filter(p => !p.selected), ...group.deselected]
  const info = tierFor({ confidence: group.confidence })

  return (
    <div style={{ padding: '8px 0 8px 12px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
        <span
          title={group.name || `Group ${index + 1}`}
          style={{
            flex: 1,
            minWidth: 0,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            textTransform: 'uppercase',
            letterSpacing: 'var(--tr-chip)',
            color: 'var(--ink-5)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          {group.name || `Group ${index + 1}`}
        </span>

        {group.resolved_by && (
          <span style={{ display: 'inline-flex', flexShrink: 0 }}>
            <Chip label={group.resolved_by} color="var(--ink-4)" title="Resolution source" />
          </span>
        )}

        {/* do not present missing confidence as a zero score. */}
        {info.hasData && (
          <>
            <ConfidencePill confidence={group.confidence} size="sm" />
            <span
              className="tabular-nums"
              style={{
                width: 30,
                textAlign: 'right',
                flexShrink: 0,
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-mono)',
                fontWeight: 600,
                color: info.color,
              }}
            >
              {info.pct}
            </span>
          </>
        )}
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 3, marginTop: 5 }}>
        {chosen.length > 0 ? (
          chosen.map((p, i) => <OptionLine key={`${p.name}-${i}`} name={p.name} on />)
        ) : (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Mark on={false} />
            <span style={{ fontSize: 'var(--fs-body)', color: 'var(--ink-5)' }}>nothing selected</span>
          </div>
        )}
      </div>

      {others.length > 0 && (
        <>
          <button
            type="button"
            className="btn"
            onClick={() => { setOpen(o => !o); }}
            aria-expanded={open}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 5,
              // prevent the block parent from stretching the button.
              width: 'fit-content',
              marginTop: 4,
              marginLeft: -6,
              padding: '2px 6px',
              border: 'none',
              background: 'transparent',
              borderRadius: 'var(--radius-chip)',
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-micro)',
              color: 'var(--ink-5)',
              textAlign: 'left',
            }}
          >
            <MIcon name={open ? 'expand_less' : 'expand_more'} size={13} />
            {others.length} other option{others.length === 1 ? '' : 's'}
          </button>
          {open && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 3, marginTop: 4 }}>
              {others.map((o, i) => (
                <OptionLine key={`${o.name}-${i}`} name={o.name} on={false} />
              ))}
            </div>
          )}
        </>
      )}
    </div>
  )
}

export default function StepsTab({ detail }: StepsTabProps) {
  const steps = detail.steps ?? []

  if (steps.length === 0) {
    return (
      <p style={{ margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-mono)', color: 'var(--ink-5)' }}>
        // no steps in this record
      </p>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      {steps.map((step, si) => {
        const groups = normalizeGroups(step)
        return (
          <section key={step.name || `step-${si}`}>
            <StepBand
              label={step.name ? `${String(si + 1).padStart(2, '0')} ${step.name}` : `Step ${si + 1}`}
              meta={`${groups.length} group${groups.length === 1 ? '' : 's'}`}
            />
            {groups.length === 0 ? (
              <p
                style={{
                  margin: 0,
                  padding: '8px 0 8px 12px',
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  color: 'var(--ink-6)',
                }}
              >
                // no groups in this step
              </p>
            ) : (
              <div style={{ marginLeft: 9 }}>
                {groups.map((group, gi) => (
                  <GroupRow key={`${group.name}-${gi}`} group={group} index={gi} />
                ))}
              </div>
            )}
          </section>
        )
      })}
    </div>
  )
}
