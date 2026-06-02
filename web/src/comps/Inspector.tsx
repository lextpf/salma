import { useState } from 'react'
import type { ReactNode } from 'react'
import Button from './Button'
import ConfidenceDial from './ConfidenceDial'
import Tabs from './Tabs'
import SpecBand from './SpecBand'
import StepsTab from './StepsTab'
import DiagnosticsTab from './DiagnosticsTab'
import { formatClockShort, formatCount, formatInferred, formatSize } from '../libraryFormat'
import type { FomodDetail, FomodEntry } from '../types'

interface InspectorProps {
  name: string | null
  priority: string | null
  entry: FomodEntry | null
  detail: FomodDetail | null
  error: string | null
  onRetry: () => void
  /** Narrow docks degrade the dial to a chip and tighten the column minimum. */
  tight?: boolean
}

// Two views, because a record answers two questions: what was selected, and how
// well that reproduced. Resist adding a third.
//   - a file manifest would repeat the virtual output tree in the column beside
//     this one, and two lists of the same files read as a duplicate
//   - a raw JSON view is a debugging surface, not a user one
//   - conflicts needs MO2 VFS data the backend does not surface, so the tab
//     would be permanently empty (see features.ts)
type TabId = 'steps' | 'diagnostics'

const TAB_ITEMS: { id: TabId; label: string }[] = [
  { id: 'steps', label: 'Selections' },
  { id: 'diagnostics', label: 'Diagnostics' },
]

/**
 * Height of the column header row, in CSS pixels, matched by the other two
 * triptych panes (records, tree) so their labels sit on one line across the
 * screen. Change one and you must change RecordsList.tsx and VfsTree.tsx too.
 */
const HEAD_H = 34

/** Tracked uppercase mono, the label voice used for every band in the module. */
const LABEL_STYLE = {
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--fs-micro)',
  letterSpacing: 'var(--tr-chip)',
  textTransform: 'uppercase',
  color: 'var(--ink-faint)',
} as const

/**
 * The inspector's column header row: the same 34px label line the records list
 * and the tree carry, so the tops of the triptych's panes line up. It carries
 * the priority as plain quiet text, because a priority is an index, not a
 * status, and an accent chip would overstate it.
 */
function HeadBand({ priority }: { priority: string | null }) {
  return (
    <div
      style={{
        ...LABEL_STYLE,
        height: HEAD_H,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 9,
        padding: '0 22px',
        letterSpacing: 'var(--tr-kicker)',
      }}
    >
      <span>Record</span>
      <span style={{ flex: 1 }} />
      <span className="tabular-nums" style={{ letterSpacing: 'var(--tr-chip)', color: 'var(--ink-5)' }}>
        {`PRI ${priority ?? '--'}`}
      </span>
    </div>
  )
}

/**
 * The inspector column for its three non-record states.
 *
 * Every state renders the header row, so the triptych's panes stay aligned at
 * the top whether a record is selected, loading or failed.
 */
function InspectorShell({
  children,
  tight,
  priority,
}: {
  children: ReactNode
  tight?: boolean
  priority: string | null
}) {
  return (
    <div
      style={{
        flex: '1 1 380px',
        minWidth: tight ? 286 : 396,
        overflow: 'hidden',
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      <HeadBand priority={priority} />
      {children}
    </div>
  )
}

/** The no-selection body: a centred placeholder on the bare plane. */
function PlaceholderBody({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        flex: 1,
        minHeight: 0,
        overflow: 'hidden',
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

// The trailing column of the Library triptych: the record inspector. It owns
// the active-tab state, which the parent resets by remounting on a record
// change, and renders the spec band plus the tabbed content. Loading and error
// for the detail are lifted to the page so the sibling VFS tree shares one
// fetch; this component only reflects them.
//
// The column carries no fill and no rules of its own. It sits on the triptych's
// single plane and stacks its regions full-bleed (header row, title, spec, tab
// strip, content), separated by spacing alone. No cards, no nested panels.
export default function Inspector({ name, priority, entry, detail, error, onRetry, tight = false }: InspectorProps) {
  const [tab, setTab] = useState<TabId>('steps')

  if (!name) {
    // A labelled placeholder rather than blank space, painting nothing of its
    // own. The metadata is static because there is no record to describe yet:
    // it says what this pane is and where its data comes from.
    return (
      <InspectorShell tight={tight} priority={null}>
        <PlaceholderBody>
          <div style={{ width: '100%', maxWidth: 340, display: 'flex', flexDirection: 'column', gap: 11 }}>
            <div style={LABEL_STYLE}>No record selected</div>
            <p style={{ margin: 0, fontSize: 'var(--fs-body)', lineHeight: 'var(--lh-body)', color: 'var(--ink-3)' }}>
              Choose a parsed FOMOD from the list to inspect its steps, diagnostics, and inferred output.
            </p>
            <div
              style={{
                paddingTop: 10,
                display: 'flex',
                flexDirection: 'column',
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-label)',
              }}
            >
              {[
                ['pane', 'inspector'],
                ['source', 'mods/<name>/fomod'],
                ['views', String(TAB_ITEMS.length)],
                ['state', 'idle'],
              ].map(([k, v]) => (
                <div key={k} style={{ display: 'flex', justifyContent: 'space-between', gap: 10 }}>
                  <span style={{ color: 'var(--ink-5)' }}>{k}</span>
                  <span style={{ color: 'var(--ink-4)', whiteSpace: 'nowrap' }}>{v}</span>
                </div>
              ))}
            </div>
          </div>
        </PlaceholderBody>
      </InspectorShell>
    )
  }

  if (error) {
    // A full-bleed band under the header rather than a centred alert card: the
    // failure is the state of this column, not an object floating in it.
    return (
      <InspectorShell tight={tight} priority={priority}>
        <div className="scroll-pane" style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}>
          <div
            style={{
              padding: '15px 22px 18px',
              background: 'var(--err-wash)',
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 10 }}>
              <span
                aria-hidden="true"
                style={{ width: 7, height: 7, borderRadius: 'var(--radius-full)', background: 'var(--danger)' }}
              />
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  letterSpacing: 'var(--tr-chip)',
                  textTransform: 'uppercase',
                  color: 'var(--danger)',
                }}
              >
                Failed to load
              </span>
              <span style={{ flex: 1 }} />
              <span
                className="tabular-nums"
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  color: 'var(--ink-5)',
                  whiteSpace: 'nowrap',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                }}
              >
                {name}
              </span>
            </div>
            <p style={{ margin: '0 0 14px', fontSize: 'var(--fs-title)', color: 'var(--ink-2)', wordBreak: 'break-word' }}>
              {error}
            </p>
            <Button icon="autorenew" label="Retry" onClick={onRetry} />
          </div>
        </div>
      </InspectorShell>
    )
  }

  if (!detail) {
    // Top-aligned and padded like the loaded layout, so the skeleton stands in
    // the place the title band and content will occupy rather than floating in
    // the middle of the column.
    return (
      <InspectorShell tight={tight} priority={priority}>
        <div style={{ flex: 1, minHeight: 0, overflow: 'hidden', padding: '16px 22px 24px' }}>
          <div style={{ width: '100%', maxWidth: 460, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div className="skeleton-line" style={{ height: 22, width: 240 }} />
            <div className="skeleton-line" style={{ height: 12, width: 320 }} />
            <div className="skeleton-line" style={{ height: 120, width: '100%', borderRadius: 'var(--radius-card)' }} />
            <div className="skeleton-line" style={{ height: 120, width: '100%', borderRadius: 'var(--radius-card)' }} />
          </div>
        </div>
      </InspectorShell>
    )
  }

  const title = detail.moduleName || name
  const stepCount = detail.steps?.length ?? entry?.stepCount ?? 0
  const stamp = detail.updated ?? detail.modified ?? entry?.modified
  const inferredText = formatInferred(stamp)

  // Everything on this line comes from the record itself. The prototype also
  // shows a mod version and an overwrite count; neither exists anywhere in the
  // schema (an overwrite is an MO2 VFS conflict, which is the same data gap
  // that removed the Conflicts tab - see the TabId note above), so they are
  // omitted rather than invented.
  const tree = detail.outputTree ?? []
  const fileCount = detail.outputTreeTotal ?? tree.length
  const installedBytes = tree.reduce((sum, f) => sum + (f.size ?? 0), 0)
  const pluginCount = tree.filter(f => /\.(esp|esm|esl)$/i.test(f.path ?? '')).length
  const truncated = detail.outputTreeTruncated === true
  const shortTime = formatClockShort(stamp)

  const meta = [
    fileCount > 0 ? `${formatCount(fileCount)} file${fileCount === 1 ? '' : 's'}${truncated ? '+' : ''}` : null,
    installedBytes > 0 && !truncated ? formatSize(installedBytes) : null,
    pluginCount > 0 && !truncated ? `${pluginCount} plugin${pluginCount === 1 ? '' : 's'}` : null,
    stepCount > 0 ? `${stepCount} step${stepCount === 1 ? '' : 's'}` : null,
    shortTime ? `scanned ${shortTime}` : null,
  ].filter(Boolean).join(' | ')

  return (
    <div
      style={{
        flex: '1 1 380px',
        minWidth: tight ? 286 : 396,
        overflow: 'hidden',
        display: 'flex',
        minHeight: 0,
      }}
    >
      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
        {/* Band 1: the column header, matching the other panes. */}
        <HeadBand priority={priority} />

        {/* Band 2: identity. */}
        <div
          style={{
            flexShrink: 0,
            padding: '16px 22px 15px',
          }}
        >
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 22 }}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8, flex: 1, minWidth: 0 }}>
              <h1
                title={title}
                style={{
                  margin: 0,
                  fontSize: 'var(--fs-h1)',
                  fontWeight: 700,
                  letterSpacing: 'var(--tr-tight)',
                  lineHeight: 1.15,
                  color: 'var(--ink)',
                  textWrap: 'pretty',
                }}
              >
                {title}
              </h1>
              <div
                title={inferredText}
                className="tabular-nums"
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-label)',
                  color: 'var(--ink-5)',
                  minWidth: 0,
                  textWrap: 'pretty',
                }}
              >
                {meta}
              </div>
            </div>
            <div style={{ flexShrink: 0 }}>
              <ConfidenceDial
                confidence={detail.diagnostics?.confidence}
                exactMatch={detail.diagnostics?.exact_match}
                compact={tight}
              />
            </div>
          </div>
        </div>

        {/* Band 3: the spec, as a band inside this column rather than a pane. */}
        <SpecBand diagnostics={detail.diagnostics} />

        {/* Band 4: the content switch. */}
        <div
          style={{
            display: 'flex',
            flexShrink: 0,
            padding: '0 22px',
          }}
        >
          <Tabs
            variant="underline"
            label="Record detail"
            items={TAB_ITEMS}
            active={tab}
            onChange={id => setTab(id as TabId)}
          />
        </div>

        <div className="scroll-pane" style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}>
          <div style={{ padding: '16px 22px 24px' }}>
            {tab === 'steps' && <StepsTab detail={detail} />}
            {tab === 'diagnostics' && <DiagnosticsTab diagnostics={detail.diagnostics} />}
          </div>
        </div>
      </div>
    </div>
  )
}
