import { useEffect, useState, useRef, useMemo, useCallback, type CSSProperties } from 'react'
import { getLogs, getTestLogs, clearLogs } from './api'
import {
  isProgressLine,
  parseProgressBars,
  fmtDur,
  fmtRate,
  type TqdmBar,
} from './progressBarParsing'
import { useVirtualScroll, ROW_LOG } from './useVirtualScroll'
import { useContentBreakpoints } from './useViewportNarrow'
import { getTailLogs } from './prefs'
import { parseLogLine, buildHistogram, facetCounts, type LogRecord } from './logParse'
import Button from './comps/Button'
import MIcon from './comps/MIcon'
import Tabs from './comps/Tabs'
import ModuleHeader from './comps/ModuleHeader'
import VolumeHistogram from './comps/VolumeHistogram'
import SubsystemFacets from './comps/SubsystemFacets'
import LogStreamRow from './comps/LogStreamRow'

/*
 * Module 03 - Logs. The data path crosses six modules, so its shape comes
 * first:
 *
 *   GET /api/logs?lines[&offset]
 *      -> { lines, nextOffset, reset, errors, warnings, passes }
 *          |                                   |
 *     loadFull (no offset)            loadIncremental (offset set)
 *     replaces the buffer             appends, trims to LINE_LIMIT,
 *          |                          adds the count deltas
 *          |                                   |
 *          |                          reset === true -> drop offset,
 *          |                          zero the counts, call loadFull
 *          +--------------> applyLines <-------+
 *                               |
 *          +--------------------+--------------------+
 *          |                                         |
 *   parseProgressBars                       records = lines minus
 *   (carries the sticky refs                tqdm progress lines, each
 *    forward, which is why                  through parseLogLine
 *    it runs here and not                        |
 *    in a useMemo)                               |
 *          |                +--------------------+--------------------+
 *   docked progress bars    |                    |                    |
 *                     facetCounts          buildHistogram        filtered by
 *                  (SubsystemFacets)     (VolumeHistogram)     level, subsystem
 *                                                              and text
 *                                                                   |
 *                                                            useVirtualScroll
 *                                                            window -> rows
 *
 * Two rules the shape enforces: applyLines is the only place `lines` changes,
 * and the sticky refs are read and written there, never during render.
 */

/**
 * How many lines to hold in the view.
 *
 * High enough to hold a whole run. A scan routinely writes five figures of log,
 * and the question a reader brings to this page is usually "what happened at the
 * start of the run", so a low ceiling drops exactly the lines that matter. The
 * engine rotates salma.log at 10 MiB, which bounds the file, so this ceiling
 * only stops an unbounded array; it is not a sampling rate.
 *
 * The stream is virtualized (useVirtualScroll), so the row count costs memory,
 * not render time.
 */
const LINE_LIMIT = 200_000
const HISTOGRAM_BUCKETS = 30
const RETRY_DELAY_MS = 2000

type LogSource = 'salma' | 'test'
type LevelFilter = 'all' | 'info' | 'warn' | 'error'

const SOURCE_TABS = [
  { id: 'salma', label: 'salma.log' },
  { id: 'test', label: 'test.log' },
]
const LEVEL_TABS = [
  { id: 'all', label: 'ALL' },
  { id: 'info', label: 'INFO' },
  { id: 'warn', label: 'WARN' },
  { id: 'error', label: 'ERROR' },
]

export default function LogsPage() {
  const [lines, setLines] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [autoRefresh, setAutoRefresh] = useState(getTailLogs)
  const [clearing, setClearing] = useState(false)
  // Set by a failed load, consumed by the retry effect below.
  const [retryPending, setRetryPending] = useState(false)
  const [source, setSource] = useState<LogSource>('salma')
  const [logStats, setLogStats] = useState({ errors: 0, warnings: 0, passes: 0 })
  // The tqdm-style progress lines filtered out of the record stream below are
  // surfaced instead as live docked bars: solver + scan progress for salma.log,
  // the test runner for test.log. This is where mod-processing progress (N/M,
  // ETA, rate) stays visible while the solver runs. Parsed on arrival, in
  // applyLines.
  const [progressBars, setProgressBars] = useState<TqdmBar[]>([])

  // Local view filters (not part of the fetch machinery).
  const [textFilter, setTextFilter] = useState('')
  const [levelFilter, setLevelFilter] = useState<LevelFilter>('all')
  const [activeSubsystem, setActiveSubsystem] = useState<string | null>(null)

  // Below 780px of content the four level tabs do not fit; they collapse to a
  // single cycling button so the filter stays reachable.
  const { compactToolbar } = useContentBreakpoints()

  const { scrollRef, handleScroll, resetScroll, stickToBottom, startIdx: getStartIdx, endIdx: getEndIdx } =
    useVirtualScroll(ROW_LOG)
  const refreshBusyRef = useRef(false)
  const abortRef = useRef<AbortController | null>(null)
  const offsetRef = useRef<number | undefined>(undefined)
  // The current `lines`, readable from the fetch path. An incremental load runs
  // in a promise handler, where reading state would give a stale buffer, and it
  // needs the buffer both to append to and to parse progress bars from.
  const linesRef = useRef<string[]>([])
  // Sticky state for the docked progress bars: the last-seen scan bar (so it
  // survives windows where no [N/M] line is in the buffer) and the per-source
  // start timestamps (so elapsed/ETA persist after the [1/N] line scrolls off).
  const cachedScanBarRef = useRef<TqdmBar | null>(null)
  const cachedScanStartTsRef = useRef<number | null>(null)
  const cachedTestStartTsRef = useRef<number | null>(null)

  // The single place `lines` changes. The progress bars are parsed here rather
  // than derived in a useMemo because parsing carries the sticky caches above
  // forward, and a render must not read or write refs.
  const applyLines = useCallback((next: string[], src: LogSource) => {
    linesRef.current = next
    setLines(next)
    const startTsRef = src === 'salma' ? cachedScanStartTsRef : cachedTestStartTsRef
    setProgressBars(next.length === 0 ? [] : parseProgressBars(next, src, cachedScanBarRef, startTsRef))
  }, [])

  // `src` is threaded through the loaders rather than read back from a ref,
  // because writing that ref during render is what react-hooks/refs forbids.
  // Neither loader touches state synchronously, so the effects below can call
  // them directly.
  const loadFull = useCallback((src: LogSource): Promise<void> => {
    refreshBusyRef.current = true
    const fetcher = src === 'test' ? getTestLogs : getLogs
    return fetcher(LINE_LIMIT)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        applyLines(data.lines, src)
        setLogStats({ errors: data.errors ?? 0, warnings: data.warnings ?? 0, passes: data.passes ?? 0 })
        offsetRef.current = data.nextOffset
        setLoading(false)
        setRetryPending(false)
      })
      .catch(e => {
        if (abortRef.current?.signal.aborted) return
        console.warn(`[logs] failed to load ${src}.log, retrying`, e)
        setLoading(true)
        setRetryPending(true)
      })
      .finally(() => {
        refreshBusyRef.current = false
      })
  }, [applyLines])

  const loadIncremental = useCallback((src: LogSource): Promise<void> => {
    if (offsetRef.current == null) return loadFull(src)
    const fetcher = src === 'test' ? getTestLogs : getLogs
    return fetcher(LINE_LIMIT, offsetRef.current)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        if (data.reset) {
          offsetRef.current = undefined
          setLogStats({ errors: 0, warnings: 0, passes: 0 })
          return loadFull(src)
        }
        offsetRef.current = data.nextOffset
        if (data.lines.length > 0) {
          const combined = [...linesRef.current, ...data.lines]
          applyLines(
            combined.length > LINE_LIMIT ? combined.slice(combined.length - LINE_LIMIT) : combined,
            src,
          )
          setLogStats(prev => ({
            errors: prev.errors + (data.errors ?? 0),
            warnings: prev.warnings + (data.warnings ?? 0),
            passes: prev.passes + (data.passes ?? 0),
          }))
        }
        setRetryPending(false)
      })
      .catch(e => {
        if (abortRef.current?.signal.aborted) return
        console.warn(`[logs] incremental load failed`, e)
      })
  }, [loadFull, applyLines])

  useEffect(() => {
    if (abortRef.current) abortRef.current.abort()
    abortRef.current = new AbortController()
    offsetRef.current = undefined
    resetScroll()
    loadFull(source)
    return () => {
      if (abortRef.current) abortRef.current.abort()
    }
  }, [source, loadFull, resetScroll])

  // A failed load comes back through here. The timer lives in an effect so that
  // its cleanup cancels it on unmount or on a source switch, and because a
  // useCallback cannot schedule a retry of itself: referencing the callback
  // inside its own body is an access before declaration.
  useEffect(() => {
    if (!retryPending) return
    const tid = setTimeout(() => {
      setRetryPending(false)
      void loadFull(source)
    }, RETRY_DELAY_MS)
    return () => clearTimeout(tid)
  }, [retryPending, loadFull, source])

  useEffect(() => {
    if (!autoRefresh) return
    let active = true
    let tid: ReturnType<typeof setTimeout>
    const poll = () => {
      loadIncremental(source).finally(() => {
        if (active) tid = setTimeout(poll, 1000)
      })
    }
    tid = setTimeout(poll, 1000)
    return () => {
      active = false
      clearTimeout(tid)
    }
  }, [autoRefresh, loadIncremental, source])

  // Parse once per lines array; progress (tqdm) lines are dropped from the stream.
  const records = useMemo<LogRecord[]>(() => {
    const out: LogRecord[] = []
    for (const line of lines) {
      if (isProgressLine(line)) continue
      out.push(parseLogLine(line))
    }
    return out
  }, [lines])

  const facets = useMemo(() => facetCounts(records), [records])
  const histogram = useMemo(() => buildHistogram(records, HISTOGRAM_BUCKETS), [records])

  const filtered = useMemo(() => {
    const q = textFilter.trim().toLowerCase()
    return records.filter(r => {
      if (levelFilter === 'info' && r.level !== 'INFO') return false
      if (levelFilter === 'warn' && r.level !== 'WARN') return false
      if (levelFilter === 'error' && r.level !== 'ERROR') return false
      if (activeSubsystem && r.subsystem !== activeSubsystem) return false
      if (q && !r.raw.toLowerCase().includes(q)) return false
      return true
    })
  }, [records, levelFilter, activeSubsystem, textFilter])

  // Keep the view pinned to the tail as new matching rows arrive.
  useEffect(() => {
    if (filtered.length === 0) return
    stickToBottom()
  }, [filtered, stickToBottom])

  // The stats and the overlay are reset here rather than in the source effect
  // above: a tab click is a plain event, and the same two calls in an effect
  // body would be cascading renders.
  const switchSource = (next: LogSource) => {
    if (next === source) return
    setLoading(true)
    setLogStats({ errors: 0, warnings: 0, passes: 0 })
    setProgressBars([])
    setRetryPending(false)
    setSource(next)
  }

  const handleClearLogs = async () => {
    setClearing(true)
    try {
      await clearLogs(source)
      offsetRef.current = undefined
      setLogStats({ errors: 0, warnings: 0, passes: 0 })
      resetScroll()
      loadFull(source)
    } catch (e) {
      console.warn(`[logs] failed to clear ${source}.log`, e)
    } finally {
      setClearing(false)
    }
  }

  const toggleSubsystem = (tag: string) => {
    setActiveSubsystem(prev => (prev === tag ? null : tag))
  }

  const totalFiltered = filtered.length
  const startIdx = getStartIdx(totalFiltered)
  const endIdx = getEndIdx(totalFiltered)
  const hasFilters = levelFilter !== 'all' || activeSubsystem != null || textFilter.trim().length > 0

  return (
    <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      <ModuleHeader num="03" label="Logs">
        <Tabs
          variant="segment"
          size="source"
          label="Log source"
          items={SOURCE_TABS}
          active={source}
          onChange={id => switchSource(id as LogSource)}
        />

        {/* A flex input with a max-width and no min-width collapses; without
            minWidth this one crushes to 53px. */}
        <div style={{ position: 'relative', flex: '1 1 200px', minWidth: 152, maxWidth: 290 }}>
          <MIcon
            name="filter_alt"
            size={15}
            style={{
              position: 'absolute',
              left: 11,
              top: '50%',
              transform: 'translateY(-50%)',
              color: 'var(--ink-5)',
              pointerEvents: 'none',
            }}
          />
          <input
            type="text"
            value={textFilter}
            onChange={e => setTextFilter(e.target.value)}
            placeholder="Filter records..."
            aria-label="Filter log records"
            style={{
              width: '100%',
              height: 30,
              padding: '0 11px 0 32px',
              border: '1px solid var(--rule-ctrl)',
              borderRadius: 'var(--radius-input)',
              background: 'var(--input)',
              color: 'var(--ink)',
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-mono)',
              outline: 'none',
            }}
          />
        </div>

        <div style={{ flex: 1 }} />

        <Button
          icon="sync"
          label="Refresh"
          compact
          onClick={() => {
            if (!refreshBusyRef.current) loadFull(source)
          }}
        />
        <Button
          icon="delete_sweep"
          label="Clear log"
          compact
          disabled={clearing}
          onClick={handleClearLogs}
        />

        {compactToolbar ? (
          <Button
            icon="tune"
            label={`Level: ${levelFilter.toUpperCase()}`}
            compact
            onClick={() => {
              const order: LevelFilter[] = ['all', 'info', 'warn', 'error']
              setLevelFilter(order[(order.indexOf(levelFilter) + 1) % order.length])
            }}
          />
        ) : (
          <Tabs
            variant="segment"
            size="level"
            label="Log level"
            items={LEVEL_TABS}
            active={levelFilter}
            onChange={id => setLevelFilter(id as LevelFilter)}
          />
        )}
      </ModuleHeader>

      {/* Volume strip */}
      <VolumeHistogram
        buckets={histogram}
        live={autoRefresh}
        onToggleLive={() => setAutoRefresh(a => !a)}
        errors={logStats.errors}
        warnings={logStats.warnings}
        passes={logStats.passes}
        showPasses={source === 'test'}
      />

      {/* Facets across the top, then the stream at full width. A log line is one
          long unbroken string, so every pixel the facets are not using is a
          pixel of message that does not need an ellipsis. */}
      <SubsystemFacets
        facets={facets}
        active={activeSubsystem}
        total={records.length}
        onToggle={toggleSubsystem}
        onClear={() => setActiveSubsystem(null)}
      />

      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
          {loading ? (
            <div style={{ padding: '10px 18px' }}>
              {[72, 58, 81, 49, 68, 55, 63].map((w, i) => (
                <div key={i} style={{ height: ROW_LOG, display: 'flex', alignItems: 'center' }}>
                  <div className="skeleton-line" style={{ height: 11, width: `${w}%` }} />
                </div>
              ))}
            </div>
          ) : totalFiltered === 0 ? (
            <div
              style={{
                flex: 1,
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                justifyContent: 'center',
                gap: 8,
                padding: 28,
                fontFamily: 'var(--font-mono)',
              }}
            >
              <span style={{ fontSize: 'var(--fs-body)', color: 'var(--ink-4)' }}>
                {records.length === 0 ? 'No log entries yet' : 'No records match the current filters'}
              </span>
              {records.length === 0 ? (
                <button
                  type="button"
                  onClick={() => loadFull(source)}
                  style={textBtnStyle}
                >
                  refresh
                </button>
              ) : hasFilters ? (
                <button
                  type="button"
                  onClick={() => {
                    setLevelFilter('all')
                    setActiveSubsystem(null)
                    setTextFilter('')
                  }}
                  style={textBtnStyle}
                >
                  clear filters
                </button>
              ) : null}
            </div>
          ) : (
            <div
              ref={scrollRef}
              onScroll={handleScroll}
              className="scroll-pane"
              style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden', padding: '8px 0' }}
            >
              <div style={{ height: startIdx * ROW_LOG }} />
              {filtered.slice(startIdx, endIdx).map((r, i) => (
                <LogStreamRow key={startIdx + i} record={r} />
              ))}
              <div style={{ height: (totalFiltered - endIdx) * ROW_LOG }} />
            </div>
          )}
        </div>
      </div>

      {/* Docked run progress (solver / scan / test). Only shown while a run is
          active; parseProgressBars returns [] once it completes. */}
      {progressBars.length > 0 && (
        <div className="log-progress-footer">
          {progressBars.map((bar, i) => (
            <SolverDock key={i} bar={bar} />
          ))}
        </div>
      )}
    </div>
  )
}

// The scan and test bars arrive as counts, but the solver hands over a
// ready-made tqdm line instead:
//   "  3%|>...................| 1.2k/40k [00:05<02:30, 238/s] | best: m=5 e=3"
// Its percentage and trailing readout are pulled back out here, or the solver
// bar would render as a permanently indeterminate hatch labelled "working".
const RAW_BAR_RE = /^\s*(\d+)%\|[^|]*\|\s*(.*)$/

/**
 * One docked run bar: a tagged phase line over a flat track.
 *
 * The numbers are the ones the engine printed; only the rendering changes, from
 * a monospace approximation of a bar to a real one. The track is a single fill
 * step and the bar is flat signal. The fill switches to the indeterminate hatch
 * only when neither counts nor a raw percentage can be parsed.
 */
function SolverDock({ bar }: { bar: TqdmBar }) {
  const raw = bar.rawBar ? RAW_BAR_RE.exec(bar.rawBar) : null
  const known = bar.current != null && bar.total != null && bar.total > 0
  const pct = known
    ? Math.min(100, Math.round((bar.current! / bar.total!) * 100))
    : raw
      ? Math.min(100, parseInt(raw[1], 10))
      : null
  const showTiming = known && bar.elapsedS != null && bar.elapsedS > 0 && bar.current! > 0
  const rate = showTiming ? bar.current! / bar.elapsedS! : 0
  const remain = showTiming && pct != null && pct < 100 ? (bar.total! - bar.current!) / rate : null
  const detail = bar.detail || raw?.[2] || 'working'

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, fontFamily: 'var(--font-mono)' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 11, fontSize: 'var(--fs-meta)' }}>
        <span style={{ color: 'var(--signal)', fontWeight: 600, flexShrink: 0 }}>
          [{bar.tag.toUpperCase()}]
        </span>
        <span
          style={{
            color: 'var(--ink-4)',
            minWidth: 0,
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          {detail}
        </span>
        <span style={{ flex: 1 }} />
        {known ? (
          <span className="tabular-nums" style={{ color: 'var(--ink-6)', flexShrink: 0, whiteSpace: 'nowrap' }}>
            {bar.current!.toLocaleString()} / {bar.total!.toLocaleString()}
            {pct != null ? ` | ${pct}%` : ''}
            {remain != null ? ` | ${fmtDur(remain)} remaining` : ''}
            {showTiming && rate >= 1 ? ` | ${fmtRate(rate)}/s` : ''}
          </span>
        ) : pct != null ? (
          <span className="tabular-nums" style={{ color: 'var(--ink-6)', flexShrink: 0, whiteSpace: 'nowrap' }}>
            {pct}%
          </span>
        ) : null}
      </div>
      <span
        style={{
          position: 'relative',
          display: 'block',
          height: 5,
          borderRadius: 'var(--radius-chip)',
          background: 'var(--track)',
          overflow: 'hidden',
        }}
      >
        <span
          className={pct == null ? 'fm-stripe' : undefined}
          style={{
            position: 'absolute',
            left: 0,
            top: 0,
            bottom: 0,
            width: `${pct ?? 100}%`,
            borderRadius: 'var(--radius-chip)',
            background: pct == null ? undefined : 'var(--signal)',
            transition: 'width 220ms ease',
          }}
        />
      </span>
    </div>
  )
}

const textBtnStyle: CSSProperties = {
  background: 'transparent',
  border: 'none',
  color: 'var(--signal-2)',
  cursor: 'pointer',
  padding: 0,
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--fs-mono)',
  textDecoration: 'underline',
  textUnderlineOffset: 3,
}
