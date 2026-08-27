import type { InstallationJob } from '../types'
import type { ConsoleLine } from '../useInstallConsole'
import MIcon from './MIcon'
import ActiveJobCard from './ActiveJobCard'
import JobLine from './JobLine'
import type { FormatSpec } from './formats'
import { ARCHIVE_FORMATS, SIDECAR, formatForFile } from './formats'
import { FormatTile } from './FormatTile'
import { STAGES } from './stages'
import { pad2 } from './jobFormat'
import { useViewportShort } from '../useViewportShort'

interface SessionFeedProps {
  jobs: InstallationJob[]
  active: { job: InstallationJob; index: number } | null
  isInstalling: boolean
  // Intake disabled (purged / unavailable / installing). Drives click-to-browse.
  locked: boolean
  purged: boolean
  consoleLines: ConsoleLine[]
  rawLines: string[]
  total: number
  onCancel: () => void
  onBrowse: () => void
  isDragging: boolean
  onDragOver: (e: React.DragEvent) => void
  onDragLeave: (e: React.DragEvent) => void
  onDrop: (e: React.DragEvent) => void
  // Best-effort names of the dragged files (usually empty: browsers hide the
  // drag payload during dragover).
  dragFileNames: string[]
  /** Library tallies, printed in the idle panel's bottom band. */
  stats?: { inferred: number; mods: number }
  /** MO2 mods directory, printed as the intake destination. */
  destPath?: string
}

/** One dotted tally in the session rule line. */
function Tally({ text, dot, fg }: { text: string; dot: string; fg: string }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, flexShrink: 0 }}>
      <span aria-hidden="true" style={{ width: 4, height: 4, borderRadius: 'var(--radius-full)', background: dot }} />
      <span className="tabular-nums" style={{ color: fg }}>{text}</span>
    </span>
  )
}

// The rail's four columns. The spine line sits at NUM_W + LINE_X, and
// everything that has to touch it (the stem under the slot, the slot's own
// glyph, the node icons) is positioned from these numbers rather than from a
// second set of guesses.
const NUM_W = 26
const SPINE_W = 22
const LINE_X = 10
const SPINE_X = NUM_W + LINE_X + 0.5

/**
 * What each stage does to the archive, as a glyph. These ride on the spine in
 * place of a plain node mark: a node has to be there anyway to say "a stage
 * happens here", so it may as well say which one.
 *
 * Indexed against STAGES, so the two have to stay in step.
 */
const STAGE_ICONS = ['shield', 'unarchive', 'manage_search', 'account_tree', 'psychology', 'drive_file_move']

/**
 * Vertical rhythm, in two sizes.
 *
 * The rail is 393px tall at its comfortable spacing, which is fine on a desktop
 * pane and too tall for a phone held sideways: an iPhone 16 Pro Max in landscape
 * is 440px, of which the top bar, module header and status bar take 140, leaving
 * a 300px feed. Rather than let a third of the pipeline sit below the fold, the
 * spacing tightens until it fits. Nothing is removed and no font shrinks - only
 * the air between the rows.
 */
interface Rhythm {
  slotPadY: number
  stem: number
  rowGap: number
  headGap: number
  footGap: number
  /** Below this, the slot's invitation folds onto one line. */
  oneLineSlot: boolean
  /** Outer padding of the scrolling column. */
  pad: string
  /** Whether the two secondary explanation lines are shown. */
  detail: boolean
}
const ROOMY: Rhythm = {
  slotPadY: 15, stem: 18, rowGap: 15, headGap: 14, footGap: 20,
  oneLineSlot: false, pad: '22px 30px 26px', detail: true,
}
// `detail: false` drops the two secondary lines, PARSE's no-fomod branch and
// INFER's fallback. They earn their space on a desktop pane and are the first
// thing to go when the choice is between them and seeing the last stage at all:
// the six stages are the content, those two sentences are footnotes.
const TIGHT: Rhythm = {
  slotPadY: 8, stem: 10, rowGap: 7, headGap: 8, footGap: 10,
  oneLineSlot: true, pad: '10px 16px 12px', detail: false,
}

/** The 1px vertical line, drawn at the spine's x for whatever height it is given. */
function SpineLine({ height }: { height?: number }) {
  return (
    <span
      aria-hidden="true"
      style={{
        position: 'absolute',
        left: LINE_X,
        top: 0,
        height: height ?? undefined,
        bottom: height == null ? 0 : undefined,
        width: 1,
        background: 'var(--rule)',
      }}
    />
  )
}

/**
 * One stage of the install pipeline: its ordinal, its glyph riding on the spine,
 * its name, and what it does to the archive.
 *
 * `last` stops the line at the node instead of running it to the row's floor -
 * the pipeline ends at WRITE, and a line continuing past it would promise a
 * seventh stage. The node sits on a plane-coloured pad so the spine passes
 * behind it cleanly rather than striking through the glyph.
 */
function StageRow({
  num,
  name,
  icon,
  gap,
  children,
  last = false,
}: {
  num: number
  name: string
  icon: string
  /** Space under the row; tightens on a short viewport. */
  gap: number
  children: React.ReactNode
  last?: boolean
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', minWidth: 0 }}>
      <span
        className="tabular-nums"
        style={{
          width: NUM_W,
          flexShrink: 0,
          paddingTop: 2,
          paddingRight: 10,
          textAlign: 'right',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          color: 'var(--ink-faint)',
        }}
      >
        {pad2(num)}
      </span>

      <span style={{ position: 'relative', width: SPINE_W, flexShrink: 0, alignSelf: 'stretch' }}>
        <SpineLine height={last ? 10 : undefined} />
        <span
          aria-hidden="true"
          style={{
            position: 'absolute',
            left: 1,
            top: 0,
            width: 19,
            height: 19,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: 'var(--paper)',
            color: 'var(--ink-faint)',
          }}
        >
          <MIcon name={icon} size={17} />
        </span>
      </span>

      <span
        style={{
          width: 84,
          flexShrink: 0,
          paddingLeft: 12,
          paddingTop: 1,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-label)',
          fontWeight: 600,
          letterSpacing: 'var(--tr-stage)',
          color: 'var(--ink-3)',
        }}
      >
        {name}
      </span>

      <div
        style={{
          flex: 1,
          minWidth: 0,
          paddingBottom: last ? 0 : gap,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-meta)',
          lineHeight: 1.55,
          color: 'var(--ink-5)',
        }}
      >
        {children}
      </div>
    </div>
  )
}

/**
 * A stage's facts, each led by its own glyph.
 *
 * A glyph per fact gives the eye an anchor and says what kind of fact it is (a
 * limit, a destination, an ordering) before the words are read. Run together
 * with middots instead, the same facts read as one grey string.
 */
function Facts({ items }: { items: { icon: string; text: string }[] }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 16, flexWrap: 'wrap' }}>
      {items.map(f => (
        <span key={f.text} style={{ display: 'inline-flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
          <MIcon name={f.icon} size={14} style={{ color: 'var(--ink-faint)', flexShrink: 0 }} />
          <span>{f.text}</span>
        </span>
      ))}
    </span>
  )
}

/** A format tile with its extension beside it, as the rail spells a format. */
function RouteTile({ spec }: { spec: FormatSpec }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, flexShrink: 0 }}>
      <FormatTile spec={spec} size={16} />
      <span style={{ fontWeight: 600, color: spec.tone }}>{spec.extension}</span>
    </span>
  )
}

export default function SessionFeed(props: SessionFeedProps) {
  const {
    jobs,
    active,
    isInstalling,
    locked,
    purged,
    consoleLines,
    rawLines,
    total,
    onCancel,
    onBrowse,
    isDragging,
    onDragOver,
    onDragLeave,
    onDrop,
    dragFileNames,
    stats,
    destPath,
  } = props

  const now = new Date()
  const sessionDate = `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())}`

  const done = jobs.filter(j => j.status === 'completed').length
  const failed = jobs.filter(j => j.status === 'error').length
  const running = jobs.filter(j => j.status === 'uploading' || j.status === 'processing').length
  const queued = jobs.filter(j => j.status === 'pending').length

  const isEmpty = jobs.length === 0

  // 620 rather than the hook's 760 default: the rail only needs to tighten when
  // the feed pane itself is short, and 760 would compact ordinary laptop panes
  // that have room to spare.
  const r: Rhythm = useViewportShort(620) ? TIGHT : ROOMY

  const browse = (e: React.MouseEvent | React.KeyboardEvent) => {
    e.stopPropagation()
    if (!locked) onBrowse()
  }

  const onBrowseKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      browse(e)
    }
  }

  // Each tally carries its own dot colour so the state of the session reads at
  // a glance instead of as one grey run of numbers.
  const ruleLine = (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-label)',
        color: 'var(--ink-6)',
      }}
    >
      <span style={{ flexShrink: 0, color: 'var(--ink-4)' }}>session {sessionDate}</span>
      <span aria-hidden="true" style={{ flex: 1, height: 1, background: 'var(--rule)' }} />
      <Tally
        text={`${jobs.length} job${jobs.length === 1 ? '' : 's'}`}
        dot="var(--meter-bar)"
        fg={isEmpty ? 'var(--ink-5)' : 'var(--ink-4)'}
      />
      {done > 0 && <Tally text={`${done} done`} dot="var(--moss)" fg="var(--moss)" />}
      {running > 0 && <Tally text={`${running} running`} dot="var(--signal)" fg="var(--signal)" />}
      {queued > 0 && <Tally text={`${queued} queued`} dot="var(--ink-faint)" fg="var(--ink-5)" />}
      {failed > 0 && <Tally text={`${failed} failed`} dot="var(--danger)" fg="var(--danger)" />}
    </div>
  )

  const caret = purged ? (
    <div
      style={{
        marginTop: 22,
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-mono)',
      }}
    >
      <MIcon name="lock" size={14} style={{ color: 'var(--ink-5)' }} />
      <span style={{ color: 'var(--ink-5)' }}>locked - deploy the plugin to continue</span>
    </div>
  ) : (
    <div
      role={locked ? undefined : 'button'}
      tabIndex={locked ? undefined : 0}
      onClick={locked ? undefined : browse}
      onKeyDown={locked ? undefined : onBrowseKey}
      style={{
        marginTop: 22,
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        cursor: locked ? 'default' : 'pointer',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-mono)',
      }}
    >
      <MIcon name="chevron_right" size={14} style={{ color: 'var(--signal)' }} />
      <span
        aria-hidden="true"
        className="caret"
        style={{
          display: 'inline-block',
          width: 8,
          height: 15,
          background: 'var(--signal)',
        }}
      />
      <span style={{ color: 'var(--ink-5)' }}>awaiting archive - drop anywhere, or click to browse</span>
    </div>
  )

  const intakeState = purged ? 'locked' : locked ? 'offline' : 'ready'
  const intakeTone = purged ? 'var(--danger)' : locked ? 'var(--ink-5)' : 'var(--signal)'

  const labelStyle: React.CSSProperties = {
    flexShrink: 0,
    fontFamily: 'var(--font-mono)',
    fontSize: 'var(--fs-micro)',
    textTransform: 'uppercase',
    letterSpacing: 'var(--tr-kicker)',
    color: 'var(--ink-faint)',
  }

  // The idle state draws the install pipeline as a rail with the drop slot at
  // its head: the same six stages the live StageMeter fills, in the same order
  // and under the same names, plus what each one does to the archive. Idle and
  // running describe one machine, not two screens.
  //
  // The format tiles sit at the extract stage rather than in a legend, because
  // that is where the extension decides something (`format_of` in
  // archive_service.rs), and a tile is how a format is spelled everywhere else
  // in salma. Which of the three readers opens which is engine detail and is
  // deliberately not printed. At the parse stage a missing fomod/ folder leaves
  // the pipeline for a content-root copy, and that is the one branch the rail
  // draws.
  //
  // Every fact hangs off the stage that enforces it. Plugin state belongs to
  // the header's state word and session totals to the rule line above, so
  // neither is repeated here.
  //
  // Centred with `margin: auto`, not `justifyContent`. The pane scrolls, and a
  // centred flex line that outgrows its container is clipped at the top with no
  // way to scroll back up to it; auto margins collapse to top-aligned instead.
  //
  // No top margin either: the auto margins have to own every pixel of slack for
  // the rail to sit in the true middle of the space under the session rule. A
  // fixed margin comes out of the top half only, and the panel then reads as
  // top-hung.
  const emptyIntake = (
    <div
      className="rise"
      // paddingTop on the container, not marginTop on the block. Auto margins
      // collapse to zero the moment the rail is taller than the pane, which is
      // what a short landscape phone hits, and the rail then sits flush against
      // the session rule. Padding survives that collapse, so there is always air
      // under the rule, and it only shifts the centred position by half its
      // value when there is room to centre.
      style={{ flex: 1, minHeight: 0, display: 'flex', paddingTop: r.detail ? 14 : 8 }}
    >
      <div style={{ width: '100%', maxWidth: 660, margin: 'auto' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 11, marginBottom: r.headGap }}>
          <span style={labelStyle}>Intake</span>
          <span aria-hidden="true" style={{ flex: 1, height: 1, background: 'var(--rule)' }} />
          <span style={{ ...labelStyle, color: intakeTone }}>{intakeState}</span>
        </div>

        {/* The slot is the only interactive part of the rail; everything below
            it is inert text. Do not put role="button" back on the panel, which
            announces the whole rail as a single control. Dropping is unaffected
            either way, because those handlers live on the region. */}
        <div
          role={locked ? undefined : 'button'}
          tabIndex={locked ? undefined : 0}
          onClick={locked ? undefined : browse}
          onKeyDown={locked ? undefined : onBrowseKey}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 14,
            // The glyph lands on the spine's x, so the archive enters the
            // machine at the top of the line it is about to travel.
            padding: `${r.slotPadY}px 18px ${r.slotPadY}px ${SPINE_X - 11}px`,
            // Dashed only while a drop would actually be taken; a locked or
            // offline intake carries a solid hairline so the dashed edge never
            // invites a refused drop (same rule as DropPromptBar).
            border: locked ? '1px solid var(--rule-ctrl)' : '1px dashed var(--signal-bd-dash)',
            borderRadius: 'var(--radius-card)',
            cursor: locked ? 'default' : 'pointer',
          }}
        >
          <MIcon name={purged ? 'lock' : 'upload'} size={22} style={{ color: 'var(--ink-4)', flexShrink: 0 }} />
          {/* Two stacked lines normally; side by side when height is scarce,
              which buys a row of the pipeline back without dropping a word. */}
          <div
            style={{
              minWidth: 0,
              display: r.oneLineSlot ? 'flex' : undefined,
              alignItems: r.oneLineSlot ? 'baseline' : undefined,
              gap: r.oneLineSlot ? 10 : undefined,
              flexWrap: r.oneLineSlot ? 'wrap' : undefined,
            }}
          >
            <div
              style={{
                fontFamily: 'var(--font-body)',
                fontSize: 'var(--fs-title)',
                fontWeight: 600,
                letterSpacing: 'var(--tr-tight)',
                color: 'var(--ink)',
              }}
            >
              {purged ? 'Intake locked' : 'Drop an archive anywhere'}
            </div>
            <div
              style={{
                marginTop: r.oneLineSlot ? 0 : 4,
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-meta)',
                lineHeight: 1.5,
                color: 'var(--ink-5)',
              }}
            >
              {purged ? (
                'deploy the plugin to continue'
              ) : locked ? (
                'connecting to mo2-server'
              ) : (
                <>
                  or{' '}
                  <span style={{ color: 'var(--signal)', textDecoration: 'underline', textUnderlineOffset: 2 }}>
                    browse
                  </span>{' '}
                  to queue one
                </>
              )}
            </div>
          </div>
        </div>

        {/* The stem, joining the slot to the first stage. */}
        <div aria-hidden="true" style={{ display: 'flex' }}>
          <span style={{ width: NUM_W, flexShrink: 0 }} />
          <span style={{ position: 'relative', width: SPINE_W, flexShrink: 0, height: r.stem }}>
            <SpineLine />
          </span>
        </div>

        <StageRow num={1} name={STAGES[0]} icon={STAGE_ICONS[0]} gap={r.rowGap}>
          <Facts
            items={[
              { icon: 'lock', text: 'path safety' },
              // The server's upload cap: kMaxUploadBytes in
              // InstallationController.cpp and kStreamThreshold in main.cpp,
              // both 8 GiB. Nothing serves that number over the API, so moving
              // it there means moving this string too.
              { icon: 'scale', text: '8 GiB max' },
            ]}
          />
        </StageRow>

        {/* Just what it opens. The tiles are the whole gloss - the stage name
            and its glyph already say what happens to them. */}
        <StageRow num={2} name={STAGES[1]} icon={STAGE_ICONS[1]} gap={r.rowGap}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 16, flexWrap: 'wrap' }}>
            {ARCHIVE_FORMATS.map(f => (
              <RouteTile key={f.extension} spec={f} />
            ))}
          </div>
        </StageRow>

        <StageRow num={3} name={STAGES[2]} icon={STAGE_ICONS[2]} gap={r.rowGap}>
          {/* The engine's 256 MiB per-entry cap is deliberately not printed
              here. It is an anti-zip-bomb guard on one file's declared size, a
              defence against forged archives rather than a limit a real install
              approaches, and in three words it reads like one. */}
          <Facts items={[{ icon: 'format_list_bulleted', text: 'file list + sizes' }]} />
        </StageRow>

        <StageRow num={4} name={STAGES[3]} icon={STAGE_ICONS[3]} gap={r.rowGap}>
          fomod/ModuleConfig.xml: steps &middot; groups &middot; plugins
          {r.detail && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginTop: 4, color: 'var(--ink-faint)' }}>
              {/* alt_route, not an arrow: this is the fork where a missing fomod
                  folder leaves the pipeline, and the glyph says fork. */}
              <MIcon name="alt_route" size={13} style={{ flexShrink: 0 }} />
              <span>no fomod: content root copied</span>
            </div>
          )}
        </StageRow>

        <StageRow num={5} name={STAGES[4]} icon={STAGE_ICONS[4]} gap={r.rowGap}>
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
            <RouteTile spec={SIDECAR} />
            <span>supplies the selections</span>
          </span>
          {r.detail && (
            <div style={{ color: 'var(--ink-faint)', marginTop: 2 }}>without one, required files only</div>
          )}
        </StageRow>

        <StageRow num={6} name={STAGES[5]} icon={STAGE_ICONS[5]} gap={r.rowGap} last>
          <Facts
            items={[
              { icon: 'folder', text: 'mods/<name>' },
              { icon: 'sort', text: 'priority order' },
            ]}
          />
        </StageRow>

        <div
          style={{
            marginTop: r.footGap,
            paddingTop: Math.round(r.footGap * 0.6),
            borderTop: '1px solid var(--rule)',
            display: 'flex',
            alignItems: 'baseline',
            gap: 18,
            flexWrap: 'wrap',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-meta)',
          }}
        >
          <span
            style={{
              minWidth: 0,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 7,
              color: destPath ? 'var(--ink-4)' : 'var(--ink-5)',
            }}
          >
            <MIcon name="folder_open" size={14} style={{ flexShrink: 0, color: 'var(--ink-faint)' }} />
            <span style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {destPath ?? 'mods path not configured'}
            </span>
          </span>
          <span style={{ flex: 1 }} />
          <span className="tabular-nums" style={{ flexShrink: 0, color: 'var(--ink-5)' }}>
            {stats ? `${stats.inferred} inferred - ${stats.mods} mods` : 'library tally unavailable'}
          </span>
        </div>
      </div>
    </div>
  )

  const history = (
    <>
      <div
        style={{
          marginTop: 26,
          marginBottom: 4,
          display: 'flex',
          alignItems: 'center',
          gap: 11,
        }}
      >
        <span style={labelStyle}>Earlier this session</span>
      </div>
      <div>
        {jobs.map((job, i) =>
          active && job.id === active.job.id ? (
            <ActiveJobCard key={job.id} job={job} index={i + 1} total={total} lines={consoleLines} rawLines={rawLines} onCancel={onCancel} />
          ) : (
            <JobLine key={job.id} job={job} index={i + 1} />
          ),
        )}
      </div>
    </>
  )

  return (
    <div
      role="region"
      aria-label="Install session feed"
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      style={{ flex: 1, minHeight: 0, position: 'relative', overflow: 'hidden', background: 'var(--paper)' }}
    >
      {!isDragging && (
        <div
          aria-hidden="true"
          style={{
            position: 'absolute',
            right: 18,
            bottom: -54,
            fontFamily: 'var(--font-body)',
            fontWeight: 800,
            fontSize: 'var(--fs-ghost)',
            lineHeight: 1,
            letterSpacing: '-0.09em',
            color: 'var(--ghost-c)',
            userSelect: 'none',
            pointerEvents: 'none',
            zIndex: 0,
          }}
        >
          01
        </div>
      )}

      <div
        style={{
          position: 'absolute',
          inset: 0,
          overflowY: 'auto',
          overflowX: 'hidden',
          opacity: isDragging ? 0.28 : 1,
          filter: isDragging ? 'grayscale(1)' : undefined,
          transition: 'opacity 120ms ease',
        }}
      >
        <div
          style={{
            minHeight: '100%',
            display: 'flex',
            flexDirection: 'column',
            padding: r.pad,
            position: 'relative',
            zIndex: 1,
          }}
        >
          {ruleLine}
          {isEmpty ? emptyIntake : history}
          {/* The idle panel already carries the invitation; a caret line under
              it would read as a second, competing one. */}
          {!isEmpty && !isInstalling && caret}
        </div>
      </div>

      {isDragging && (
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 3 }}>
          <div
            className="rise"
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              gap: 12,
              padding: '26px 40px',
              border: '1px dashed var(--signal-bd-strong)',
              borderRadius: 'var(--radius-card)',
              background: 'var(--card)',
            }}
          >
            <span
              style={{
                width: 40,
                height: 40,
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                borderRadius: 'var(--radius-panel)',
                color: 'var(--signal-ink)',
                background: 'var(--signal)',
              }}
            >
              <MIcon name="arrow_downward" size={22} />
            </span>
            <div style={{ fontFamily: 'var(--font-body)', fontSize: 'var(--fs-empty)', fontWeight: 700, letterSpacing: 'var(--tr-tight)', color: 'var(--ink)' }}>
              {dragFileNames.length > 0 ? `Release to queue ${dragFileNames.length} archive${dragFileNames.length === 1 ? '' : 's'}` : 'Release to queue'}
            </div>
            {dragFileNames.length > 0 && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', justifyContent: 'center', fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-4)' }}>
                {dragFileNames.slice(0, 4).map((name, i) => (
                  <span key={i} style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                    <FormatTile spec={formatForFile(name)} size={18} />
                    {name}
                  </span>
                ))}
              </div>
            )}
            <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-5)' }}>they run in order - the current job is never interrupted</div>
          </div>
        </div>
      )}
    </div>
  )
}
