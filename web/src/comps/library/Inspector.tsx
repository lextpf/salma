import { useState } from 'react'
import MIcon from '../MIcon'
import Tabs from '../Tabs'
import SpecRail from './SpecRail'
import StepsTab from './tabs/StepsTab'
import DiagnosticsTab from './tabs/DiagnosticsTab'
import FilesTab from './tabs/FilesTab'
import ConflictsTab from './tabs/ConflictsTab'
import JsonTab from './tabs/JsonTab'
import { formatInferred, formatSize } from '../../libraryFormat'
import type { FomodDetail, FomodEntry } from '../../types'

interface InspectorProps {
  name: string | null
  priority: string | null
  entry: FomodEntry | null
  detail: FomodDetail | null
  error: string | null
  onRetry: () => void
}

type TabId = 'steps' | 'diagnostics' | 'files' | 'conflicts' | 'json'

const TAB_ITEMS: { id: TabId; label: string }[] = [
  { id: 'steps', label: 'Steps' },
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'files', label: 'Files' },
  { id: 'conflicts', label: 'Conflicts' },
  { id: 'json', label: 'JSON' },
]

function CenteredState({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        flex: '2 1 340px',
        minWidth: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 32,
      }}
    >
      {children}
    </div>
  )
}

function MetaItem({ k, v }: { k: string; v: string }) {
  return (
    <span style={{ whiteSpace: 'nowrap' }}>
      <span style={{ color: 'var(--ink-6)' }}>{k}</span>{' '}
      <span className="tabular-nums" style={{ color: 'var(--ink)' }}>
        {v}
      </span>
    </span>
  )
}

// Column 3 of the triptych: the record inspector. Holds the active-tab state
// (reset by the parent remounting on record change), and renders the spec rail
// plus the tabbed content. Detail loading/error are lifted to the page so the
// sibling VFS tree shares the same fetch; this component just reflects them.
export default function Inspector({ name, priority, entry, detail, error, onRetry }: InspectorProps) {
  const [tab, setTab] = useState<TabId>('steps')

  if (!name) {
    return (
      <CenteredState>
        <div style={{ textAlign: 'center', maxWidth: 320 }}>
          <div
            style={{
              width: 44,
              height: 44,
              margin: '0 auto 14px',
              borderRadius: '50%',
              border: '1px solid var(--rule-strong)',
              background: 'var(--card)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--ink-4)',
            }}
          >
            <MIcon name="layers" size={22} />
          </div>
          <div style={{ fontSize: 'var(--fs-title)', fontWeight: 600, color: 'var(--ink-2)', marginBottom: 6 }}>
            Select a record
          </div>
          <p style={{ margin: 0, fontSize: 'var(--fs-body)', lineHeight: 'var(--lh-body)', color: 'var(--ink-4)' }}>
            Choose a parsed FOMOD from the list to inspect its steps, diagnostics, and inferred output.
          </p>
        </div>
      </CenteredState>
    )
  }

  if (error) {
    return (
      <CenteredState>
        <div
          style={{
            maxWidth: 420,
            padding: '20px 24px',
            background: 'var(--card)',
            border: '1px solid var(--rule)',
            borderRadius: 10,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 10 }}>
            <span
              aria-hidden="true"
              style={{ width: 7, height: 7, borderRadius: '50%', background: 'var(--danger)' }}
            />
            <span
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-micro)',
                letterSpacing: '0.14em',
                textTransform: 'uppercase',
                color: 'var(--danger)',
              }}
            >
              Failed to load
            </span>
          </div>
          <p style={{ margin: '0 0 14px', fontSize: 'var(--fs-title)', color: 'var(--ink-2)', wordBreak: 'break-word' }}>
            {error}
          </p>
          <button
            type="button"
            onClick={onRetry}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 8,
              padding: '7px 14px',
              borderRadius: 7,
              border: '1px solid var(--rule)',
              background: 'var(--sheet)',
              color: 'var(--ink-2)',
              fontSize: 'var(--fs-body)',
              fontFamily: 'inherit',
              cursor: 'pointer',
            }}
          >
            <MIcon name="autorenew" size={13} />
            <span>Retry</span>
          </button>
        </div>
      </CenteredState>
    )
  }

  if (!detail) {
    return (
      <CenteredState>
        <div style={{ width: '100%', maxWidth: 460, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div className="skeleton-line" style={{ height: 22, width: 240 }} />
          <div className="skeleton-line" style={{ height: 12, width: 320 }} />
          <div className="skeleton-line" style={{ height: 120, width: '100%', borderRadius: 9 }} />
          <div className="skeleton-line" style={{ height: 120, width: '100%', borderRadius: 9 }} />
        </div>
      </CenteredState>
    )
  }

  const title = detail.moduleName || name
  const stepCount = detail.steps?.length ?? entry?.stepCount ?? 0
  const sizeText = formatSize(entry?.size)
  const inferredText = formatInferred(detail.updated ?? detail.modified ?? entry?.modified)

  return (
    <div style={{ flex: '2 1 340px', minWidth: 0, display: 'flex', minHeight: 0, boxShadow: 'var(--shadow-elevation-2)' }}>
      <SpecRail diagnostics={detail.diagnostics} />

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
        <div style={{ flexShrink: 0, padding: '16px 20px 0', borderBottom: '1px solid var(--rule-soft)' }}>
          <div
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-micro)',
              letterSpacing: '0.1em',
              color: 'var(--ink-6)',
              marginBottom: 5,
            }}
          >
            #{priority ?? '--'} - FOMOD RECORD
          </div>
          <h1
            style={{
              margin: 0,
              fontSize: 'var(--fs-head)',
              fontWeight: 600,
              letterSpacing: '-0.02em',
              color: 'var(--ink)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {title}
          </h1>
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 14,
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-label)',
              color: 'var(--ink-4)',
              margin: '9px 0 13px',
              flexWrap: 'wrap',
            }}
          >
            <MetaItem k="steps" v={String(stepCount)} />
            <MetaItem k="size" v={sizeText} />
            <MetaItem k="inferred" v={inferredText} />
          </div>
          <Tabs variant="underline" items={TAB_ITEMS} active={tab} onChange={id => setTab(id as TabId)} />
        </div>

        <div className="scroll-pane" style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}>
          <div style={{ padding: '16px 18px' }}>
            {tab === 'steps' && <StepsTab detail={detail} />}
            {tab === 'diagnostics' && <DiagnosticsTab diagnostics={detail.diagnostics} />}
            {tab === 'files' && <FilesTab detail={detail} />}
            {tab === 'conflicts' && <ConflictsTab />}
            {tab === 'json' && <JsonTab detail={detail} name={title} />}
          </div>
        </div>
      </div>
    </div>
  )
}
