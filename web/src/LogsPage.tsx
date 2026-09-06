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

// full reads replace the buffer. incremental reads append count deltas.
// `applyLines` owns line state and progress refs. render code must not mutate those refs.
// the engine rotates logs at 10 MiB. this separate limit bounds browser memory.
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
  const [retryPending, setRetryPending] = useState(false)
  const [source, setSource] = useState<LogSource>('salma')
  const [logStats, setLogStats] = useState({ errors: 0, warnings: 0, passes: 0 })
  // progress records render in the footer and do not enter the record stream.
  const [progressBars, setProgressBars] = useState<TqdmBar[]>([])

  const [textFilter, setTextFilter] = useState('')
  const [levelFilter, setLevelFilter] = useState<LevelFilter>('all')
  const [activeSubsystem, setActiveSubsystem] = useState<string | null>(null)

  const { compactToolbar } = useContentBreakpoints()

  const { scrollRef, handleScroll, resetScroll, stickToBottom, startIdx: getStartIdx, endIdx: getEndIdx } =
    useVirtualScroll(ROW_LOG)
  const refreshBusyRef = useRef(false)
  const abortRef = useRef<AbortController | null>(null)
  const offsetRef = useRef<number | undefined>(undefined)
  // asynchronous appends read the current buffer through this ref.
  const linesRef = useRef<string[]>([])
  // keep progress state after source records leave the bounded buffer.
  const cachedScanBarRef = useRef<TqdmBar | null>(null)
  const cachedScanStartTsRef = useRef<number | null>(null)
  const cachedTestStartTsRef = useRef<number | null>(null)

  // update line state and progress refs together.
  const applyLines = useCallback((next: string[], src: LogSource) => {
    linesRef.current = next
    setLines(next)
    const startTsRef = src === 'salma' ? cachedScanStartTsRef : cachedTestStartTsRef
    setProgressBars(next.length === 0 ? [] : parseProgressBars(next, src, cachedScanBarRef, startTsRef))
  }, [])

  const loadFull = useCallback((src: LogSource): Promise<void> => {
    refreshBusyRef.current = true
    const fetcher = src === 'test' ? getTestLogs : getLogs
    return fetcher(LINE_LIMIT)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        applyLines(data.lines, src)
        setLogStats({ errors: data.errors, warnings: data.warnings, passes: data.passes })
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
            errors: prev.errors + data.errors,
            warnings: prev.warnings + data.warnings,
            passes: prev.passes + data.passes,
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
    void loadFull(source)
    return () => {
      if (abortRef.current) abortRef.current.abort()
    }
  }, [source, loadFull, resetScroll])

  // keep retry timers in an effect so cleanup cancels them.
  useEffect(() => {
    if (!retryPending) return
    const tid = setTimeout(() => {
      setRetryPending(false)
      void loadFull(source)
    }, RETRY_DELAY_MS)
    return () => { clearTimeout(tid); }
  }, [retryPending, loadFull, source])

  useEffect(() => {
    if (!autoRefresh) return
    let active = true
    let tid: ReturnType<typeof setTimeout>
    const poll = () => {
      void loadIncremental(source).finally(() => {
        if (active) tid = setTimeout(poll, 1000)
      })
    }
    tid = setTimeout(poll, 1000)
    return () => {
      active = false
      clearTimeout(tid)
    }
  }, [autoRefresh, loadIncremental, source])

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

  useEffect(() => {
    if (filtered.length === 0) return
    stickToBottom()
  }, [filtered, stickToBottom])

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
      void loadFull(source)
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
          onChange={id => { switchSource(id as LogSource); }}
        />

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
            onChange={e => { setTextFilter(e.target.value); }}
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
            if (!refreshBusyRef.current) void loadFull(source)
          }}
        />
        <Button
          icon="delete_sweep"
          label="Clear log"
          compact
          disabled={clearing}
          onClick={() => { void handleClearLogs(); }}
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
            onChange={id => { setLevelFilter(id as LevelFilter); }}
          />
        )}
      </ModuleHeader>

      <VolumeHistogram
        buckets={histogram}
        live={autoRefresh}
        onToggleLive={() => { setAutoRefresh(a => !a); }}
        errors={logStats.errors}
        warnings={logStats.warnings}
        passes={logStats.passes}
        showPasses={source === 'test'}
      />

      <SubsystemFacets
        facets={facets}
        active={activeSubsystem}
        total={records.length}
        onToggle={toggleSubsystem}
        onClear={() => { setActiveSubsystem(null); }}
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
                  onClick={() => { void loadFull(source); }}
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

// solver progress arrives as a preformatted tqdm line instead of counts.
const RAW_BAR_RE = /^\s*(\d+)%\|[^|]*\|\s*(.*)$/

// use an indeterminate fill only when neither counts nor a raw percentage exists.
function SolverDock({ bar }: { bar: TqdmBar }) {
  // bind the optional counts to consts so `known` and `showTiming` narrow them everywhere.
  const { current, total, elapsedS } = bar
  const raw = bar.rawBar ? RAW_BAR_RE.exec(bar.rawBar) : null
  const known = current != null && total != null && total > 0
  const pct = known
    ? Math.min(100, Math.round((current / total) * 100))
    : raw
      ? Math.min(100, parseInt(raw[1], 10))
      : null
  const showTiming = known && elapsedS != null && elapsedS > 0 && current > 0
  const rate = showTiming ? current / elapsedS : 0
  const remain = showTiming && pct != null && pct < 100 ? (total - current) / rate : null
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
            {current.toLocaleString()} / {total.toLocaleString()}
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
