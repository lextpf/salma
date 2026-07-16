import { useEffect, useState, useRef, useMemo, useCallback, type CSSProperties } from 'react'
import { getLogs, getTestLogs, clearLogs } from './api'
import {
  isProgressLine,
  parseProgressBars,
  highlightRawBar,
  renderTqdmBar,
  type TqdmBar,
} from './progressBarParsing'
import { useVirtualScroll, LINE_HEIGHT } from './useVirtualScroll'
import { parseLogLine, buildHistogram, facetCounts, type LogRecord } from './logParse'
import Kicker from './comps/Kicker'
import Tabs from './comps/Tabs'
import MIcon from './comps/MIcon'
import VolumeHistogram from './comps/logs/VolumeHistogram'
import SubsystemFacets from './comps/logs/SubsystemFacets'
import LogStreamRow from './comps/logs/LogStreamRow'

const LINE_LIMIT = 1000
const HISTOGRAM_BUCKETS = 12
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
  const [autoRefresh, setAutoRefresh] = useState(true)
  const [clearing, setClearing] = useState(false)
  const [source, setSource] = useState<LogSource>('salma')
  const [logStats, setLogStats] = useState({ errors: 0, warnings: 0, passes: 0 })

  // Local view filters (not part of the fetch machinery).
  const [textFilter, setTextFilter] = useState('')
  const [levelFilter, setLevelFilter] = useState<LevelFilter>('all')
  const [activeSubsystem, setActiveSubsystem] = useState<string | null>(null)

  const { scrollRef, scrollEl, handleScroll, isAtBottomRef, resetScroll, startIdx: getStartIdx, endIdx: getEndIdx } =
    useVirtualScroll()
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const refreshBusyRef = useRef(false)
  const abortRef = useRef<AbortController | null>(null)
  const offsetRef = useRef<number | undefined>(undefined)
  // Sticky state for the docked progress bars: the last-seen scan bar (so it
  // survives windows where no [N/M] line is in the buffer) and the per-source
  // start timestamps (so elapsed/ETA persist after the [1/N] line scrolls off).
  const cachedScanBarRef = useRef<TqdmBar | null>(null)
  const cachedScanStartTsRef = useRef<number | null>(null)
  const cachedTestStartTsRef = useRef<number | null>(null)
  const sourceRef = useRef(source)
  sourceRef.current = source

  const clearRetryTimer = () => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current)
      retryTimerRef.current = null
    }
  }

  const scheduleRetry = () => {
    if (retryTimerRef.current) return
    retryTimerRef.current = setTimeout(() => {
      retryTimerRef.current = null
      void loadFull(false)
    }, RETRY_DELAY_MS)
  }

  const loadFull = useCallback((showBusy = false) => {
    if (showBusy) setLoading(true)
    refreshBusyRef.current = true
    const currentSource = sourceRef.current
    const fetcher = currentSource === 'test' ? getTestLogs : getLogs
    return fetcher(LINE_LIMIT)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        setLines(data.lines)
        setLogStats({ errors: data.errors ?? 0, warnings: data.warnings ?? 0, passes: data.passes ?? 0 })
        offsetRef.current = data.nextOffset
        setLoading(false)
        clearRetryTimer()
      })
      .catch(e => {
        if (abortRef.current?.signal.aborted) return
        console.warn(`[logs] failed to load ${currentSource}.log, retrying`, e)
        setLoading(true)
        scheduleRetry()
      })
      .finally(() => {
        refreshBusyRef.current = false
      })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const loadIncremental = useCallback((): Promise<void> => {
    if (offsetRef.current == null) return loadFull()
    const currentSource = sourceRef.current
    const fetcher = currentSource === 'test' ? getTestLogs : getLogs
    return fetcher(LINE_LIMIT, offsetRef.current)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        if (data.reset) {
          offsetRef.current = undefined
          setLogStats({ errors: 0, warnings: 0, passes: 0 })
          return loadFull()
        }
        offsetRef.current = data.nextOffset
        if (data.lines.length > 0) {
          setLines(prev => {
            const combined = [...prev, ...data.lines]
            if (combined.length > LINE_LIMIT) {
              return combined.slice(combined.length - LINE_LIMIT)
            }
            return combined
          })
          setLogStats(prev => ({
            errors: prev.errors + (data.errors ?? 0),
            warnings: prev.warnings + (data.warnings ?? 0),
            passes: prev.passes + (data.passes ?? 0),
          }))
        }
        clearRetryTimer()
      })
      .catch(e => {
        if (abortRef.current?.signal.aborted) return
        console.warn(`[logs] incremental load failed`, e)
      })
  }, [loadFull])

  useEffect(() => {
    if (abortRef.current) abortRef.current.abort()
    abortRef.current = new AbortController()
    offsetRef.current = undefined
    setLogStats({ errors: 0, warnings: 0, passes: 0 })
    resetScroll()
    void loadFull(true)
    return () => {
      if (abortRef.current) abortRef.current.abort()
    }
  }, [source, loadFull, resetScroll])

  useEffect(() => {
    if (!autoRefresh) return
    let active = true
    let tid: ReturnType<typeof setTimeout>
    const poll = () => {
      loadIncremental().finally(() => {
        if (active) tid = setTimeout(poll, 1000)
      })
    }
    tid = setTimeout(poll, 1000)
    return () => {
      active = false
      clearTimeout(tid)
    }
  }, [autoRefresh, loadIncremental])

  useEffect(() => clearRetryTimer, [])

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

  // The tqdm-style progress lines filtered out of the record stream above are
  // surfaced here instead as live docked bars: solver + scan progress for
  // salma.log, the test runner for test.log. This is where mod-processing
  // progress (N/M, ETA, rate) stays visible while the solver runs.
  const progressBars = useMemo(() => {
    if (loading || lines.length === 0) return []
    const startTsRef = source === 'salma' ? cachedScanStartTsRef : cachedTestStartTsRef
    return parseProgressBars(lines, source, cachedScanBarRef, startTsRef)
  }, [lines, loading, source])

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
    if (!isAtBottomRef.current) return
    const el = scrollEl.current
    if (el) el.scrollTop = el.scrollHeight
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filtered, scrollEl])

  const handleClearLogs = async () => {
    setClearing(true)
    try {
      await clearLogs(source)
      offsetRef.current = undefined
      setLogStats({ errors: 0, warnings: 0, passes: 0 })
      resetScroll()
      void loadFull()
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
      {/* Header bar */}
      <div
        style={{
          height: 46,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '0 18px',
          borderBottom: '1px solid var(--rule-soft)',
          boxShadow: 'var(--shadow-elevation-1)',
        }}
      >
        <Kicker num="03" label="Logs" />

        <Tabs variant="pill" items={SOURCE_TABS} active={source} onChange={id => { setSource(id as LogSource); }} />

        <div style={{ position: 'relative', flex: 1, maxWidth: 300 }}>
          <input
            type="text"
            value={textFilter}
            onChange={e => { setTextFilter(e.target.value); }}
            placeholder="Filter records..."
            aria-label="Filter log records"
            style={{
              width: '100%',
              padding: '7px 11px',
              border: '1px solid var(--rule)',
              borderRadius: 7,
              background: 'var(--sheet)',
              color: 'var(--ink)',
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-label)',
              outline: 'none',
            }}
          />
        </div>

        <div style={{ flex: 1 }} />

        <button
          type="button"
          onClick={() => {
            if (!refreshBusyRef.current) void loadFull(false)
          }}
          aria-label="Refresh log"
          title="Refresh"
          style={iconBtnStyle}
        >
          <MIcon name="sync" size={13} />
        </button>
        <button
          type="button"
          onClick={handleClearLogs}
          disabled={clearing}
          aria-label="Clear log"
          title="Clear log"
          style={{ ...iconBtnStyle, opacity: clearing ? 0.5 : 1 }}
        >
          <MIcon name="delete_sweep" size={13} />
        </button>

        <Tabs variant="pill" items={LEVEL_TABS} active={levelFilter} onChange={id => { setLevelFilter(id as LevelFilter); }} />
      </div>

      {/* Volume strip */}
      <VolumeHistogram
        buckets={histogram}
        live={autoRefresh}
        onToggleLive={() => { setAutoRefresh(a => !a); }}
        errors={logStats.errors}
        warnings={logStats.warnings}
        passes={logStats.passes}
        showPasses={source === 'test'}
      />

      {/* Dual pane: subsystem facets + stream */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <SubsystemFacets
          facets={facets}
          active={activeSubsystem}
          total={records.length}
          onToggle={toggleSubsystem}
          onClear={() => { setActiveSubsystem(null); }}
        />

        <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
          {loading ? (
            <div style={{ padding: '10px 18px' }}>
              {[72, 58, 81, 49, 68, 55, 63].map((w, i) => (
                <div key={i} style={{ height: LINE_HEIGHT, display: 'flex', alignItems: 'center' }}>
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
                  onClick={() => loadFull(false)}
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
              <div style={{ height: startIdx * LINE_HEIGHT }} />
              {filtered.slice(startIdx, endIdx).map((r, i) => (
                <LogStreamRow key={startIdx + i} record={r} />
              ))}
              <div style={{ height: (totalFiltered - endIdx) * LINE_HEIGHT }} />
            </div>
          )}
        </div>
      </div>

      {/* Docked tqdm progress bars (solver / scan / test). Only shown while a
          run is active; parseProgressBars returns [] once it completes. */}
      {progressBars.length > 0 && (
        <div className="log-progress-footer">
          {progressBars.map((bar, i) => (
            <div key={i} className="log-progress-bar-line">
              <span className="log-tag">[{bar.tag}]</span>{' '}
              {bar.rawBar
                ? highlightRawBar(bar.rawBar)
                : bar.current != null && bar.total != null
                  ? renderTqdmBar(bar.current, bar.total, bar.detail, bar.elapsedS)
                  : null}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

const iconBtnStyle: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  width: 28,
  height: 28,
  border: '1px solid var(--rule)',
  borderRadius: 7,
  background: 'var(--sheet)',
  color: 'var(--ink-3)',
  cursor: 'pointer',
}

const textBtnStyle: CSSProperties = {
  background: 'transparent',
  border: 'none',
  color: 'var(--ink-3)',
  cursor: 'pointer',
  padding: 0,
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--fs-label)',
  textDecoration: 'underline',
  textUnderlineOffset: 2,
}
