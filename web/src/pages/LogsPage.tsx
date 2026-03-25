import { useEffect, useState, useRef, useMemo, useCallback } from 'react'
import { getLogs, getTestLogs, clearLogs } from '../api'
import { isProgressLine, parseProgressBars, highlightRawBar, renderTqdmBar, TqdmBar } from '../utils/progressBarParsing'
import { LogLine } from '../components/LogLine'
import { useVirtualScroll, LINE_HEIGHT } from '../hooks/useVirtualScroll'

const LINE_OPTIONS = [50, 100, 200, 500, 1000, 0] as const
const RETRY_DELAY_MS = 2000

type LogSource = 'salma' | 'test'

export default function LogsPage() {
  const [lines, setLines] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [autoRefresh, setAutoRefresh] = useState(true)
  const [heartBeatTick, setHeartBeatTick] = useState(0)
  const [lineCount, setLineCount] = useState(1000)
  const [dropdownOpen, setDropdownOpen] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [source, setSource] = useState<LogSource>('salma')
  const [logStats, setLogStats] = useState({ errors: 0, warnings: 0, passes: 0 })
  const { scrollRef, handleScroll, isAtBottomRef, resetScroll, startIdx: getStartIdx, endIdx: getEndIdx } = useVirtualScroll()
  const dropdownRef = useRef<HTMLDivElement>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)
  const abortRef = useRef<AbortController | null>(null)
  const offsetRef = useRef<number | undefined>(undefined)
  const cachedScanBarRef = useRef<TqdmBar | null>(null)
  const cachedScanStartTsRef = useRef<number | null>(null)
  const cachedTestStartTsRef = useRef<number | null>(null)
  const sourceRef = useRef(source)
  sourceRef.current = source
  const lineCountRef = useRef(lineCount)
  lineCountRef.current = lineCount

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
      loadFull(false, true)
    }, RETRY_DELAY_MS)
  }

  // Close dropdown on outside click
  useEffect(() => {
    if (!dropdownOpen) return
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [dropdownOpen])

  // Full load: fetches last N lines, replaces state entirely
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const loadFull = useCallback((showBusy = false, force = false) => {
    if (inFlightRef.current && !force) return
    if (showBusy) setLoading(true)
    inFlightRef.current = true
    const currentSource = sourceRef.current
    const currentLineCount = lineCountRef.current
    const fetcher = currentSource === 'test' ? getTestLogs : getLogs
    fetcher(currentLineCount)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        setLines(data.lines)
        setLogStats({ errors: data.errors ?? 0, warnings: data.warnings ?? 0, passes: data.passes ?? 0 })
        offsetRef.current = data.nextOffset
        setLoading(false)
        clearRetryTimer()
        setHeartBeatTick(t => t + 1)
      })
      .catch(e => {
        if (abortRef.current?.signal.aborted) return
        console.warn(`[logs] failed to load ${currentSource}.log, retrying`, e)
        setLoading(true)
        scheduleRetry()
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }, [])

  // Incremental load: fetches only new lines since last offset
  const loadIncremental = useCallback(() => {
    if (inFlightRef.current) return
    if (offsetRef.current == null) { loadFull(); return }
    inFlightRef.current = true
    const currentSource = sourceRef.current
    const currentLineCount = lineCountRef.current
    const fetcher = currentSource === 'test' ? getTestLogs : getLogs
    fetcher(currentLineCount, offsetRef.current)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        if (data.reset) {
          // Log was cleared - do a full reload
          offsetRef.current = undefined
          setLogStats({ errors: 0, warnings: 0, passes: 0 })
          cachedScanBarRef.current = null
          cachedScanStartTsRef.current = null
          cachedTestStartTsRef.current = null
          inFlightRef.current = false
          loadFull()
          return
        }
        offsetRef.current = data.nextOffset
        if (data.lines.length > 0) {
          setLines(prev => {
            const combined = [...prev, ...data.lines]
            // If lineCount is set (not "all"), trim from front
            if (currentLineCount > 0 && combined.length > currentLineCount) {
              return combined.slice(combined.length - currentLineCount)
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
        setHeartBeatTick(t => t + 1)
      })
      .catch(e => {
        if (abortRef.current?.signal.aborted) return
        console.warn(`[logs] incremental load failed`, e)
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }, [loadFull])

  // Reset offset when source or lineCount changes
  useEffect(() => {
    if (abortRef.current) abortRef.current.abort()
    abortRef.current = new AbortController()
    inFlightRef.current = false
    offsetRef.current = undefined
    setLogStats({ errors: 0, warnings: 0, passes: 0 })
    cachedScanBarRef.current = null
    resetScroll()
    loadFull(true)
    return () => {
      if (abortRef.current) abortRef.current.abort()
    }
  }, [lineCount, source, loadFull])

  useEffect(() => {
    if (!autoRefresh) return
    const id = setInterval(loadIncremental, 1000)
    return () => clearInterval(id)
  }, [autoRefresh, lineCount, source, loadIncremental])

  useEffect(() => {
    if (!isAtBottomRef.current) return
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [lines])

  useEffect(() => clearRetryTimer, [])

  const filteredLines = useMemo(() => {
    const result: string[] = []
    let i = 0
    while (i < lines.length) {
      if (isProgressLine(lines[i])) {
        let runEnd = i + 1
        while (runEnd < lines.length && isProgressLine(lines[runEnd])) runEnd++
        i = runEnd
      } else {
        result.push(lines[i])
        i++
      }
    }
    return result
  }, [lines])

  const progressBars = useMemo(() => {
    if (loading || lines.length === 0) return []
    const startTsRef = source === 'salma' ? cachedScanStartTsRef : cachedTestStartTsRef
    return parseProgressBars(lines, source, cachedScanBarRef, startTsRef)
  }, [lines, loading, source])

  const handleClearLogs = async () => {
    setClearing(true)
    try {
      await clearLogs(source)
      offsetRef.current = undefined
      setLogStats({ errors: 0, warnings: 0, passes: 0 })
      cachedScanBarRef.current = null
      cachedScanStartTsRef.current = null
      cachedTestStartTsRef.current = null
      resetScroll()
      loadFull()
    } catch (e) {
      console.warn(`[logs] failed to clear ${source}.log`, e)
    } finally {
      setClearing(false)
    }
  }

  const totalFiltered = filteredLines.length
  const startIdx = getStartIdx(totalFiltered)
  const endIdx = getEndIdx(totalFiltered)

  return (
    <div className="animate-fade-in">
      <header className="page-header page-header-logs mb-6">
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="page-title text-2xl font-bold text-on-surface flex items-center gap-3 shrink-0">
            <span className="inline-flex w-6 shrink-0 items-center justify-center">
              <i className="fa-duotone fa-solid fa-terminal icon-gradient icon-gradient-ember text-2xl" />
            </span>
            Logs
          </h1>
        </div>

        <p className="page-subtitle mt-1 text-sm ml-9 flex flex-wrap items-center gap-1.5">
          <span>Inspect logs in real time</span>
          <button
            type="button"
            onClick={() => setSource('salma')}
            className={`text-xs transition-colors underline-offset-2 ${
              source === 'salma'
                ? 'text-primary underline'
                : 'text-on-surface-variant hover:text-primary'
            }`}
            aria-pressed={source === 'salma'}
          >
            salma.log
          </button>
          <span className="text-outline/70">|</span>
          <button
            type="button"
            onClick={() => setSource('test')}
            className={`text-xs transition-colors underline-offset-2 ${
              source === 'test'
                ? 'text-primary underline'
                : 'text-on-surface-variant hover:text-primary'
            }`}
            aria-pressed={source === 'test'}
          >
            test.log
          </button>
        </p>
      </header>

      {loading ? (
        <div className="mb-4 flex flex-wrap items-center gap-2">
          <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
          <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
          <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
        </div>
      ) : (
        <div className="mb-4 flex flex-wrap items-center gap-2">
          <div className="relative" ref={dropdownRef}>
            <button
              onClick={() => setDropdownOpen(o => !o)}
              className="rounded-lg border border-outline-variant/60 bg-surface-container px-3 py-1.5 text-xs text-on-surface
                         hover:bg-surface-container-high hover:border-primary/40 transition-all flex items-center gap-2"
            >
              <span className="tabular-nums font-medium">{lineCount === 0 ? 'All' : lineCount}</span>
              <span className="text-on-surface-variant">lines</span>
              <i className={`fa-duotone fa-solid fa-chevron-down text-[0.5rem] text-outline transition-transform duration-200 ${dropdownOpen ? 'rotate-180' : ''}`} />
            </button>
            {dropdownOpen && (
              <div className="absolute left-0 top-full mt-1.5 py-1.5 rounded-xl overflow-hidden bg-surface-container-high border border-outline-variant/40 shadow-elevation-3 z-50 min-w-[7rem] animate-fade-in">
                {LINE_OPTIONS.map(n => (
                  <button
                    key={n}
                    onClick={() => { setLineCount(n); setDropdownOpen(false) }}
                    className={`w-full text-left px-3.5 py-2 text-xs transition-colors flex items-center justify-between
                      ${n === lineCount
                        ? 'text-primary bg-primary/8 font-medium'
                        : 'text-on-surface-variant hover:text-on-surface hover:bg-surface-container-highest'
                      }`}
                  >
                    <span className="tabular-nums">{n === 0 ? 'All' : n}</span>
                    {n === lineCount && <i className="fa-duotone fa-solid fa-check text-[0.55rem] text-primary" />}
                  </button>
                ))}
              </div>
            )}
          </div>

          <button
            onClick={() => setAutoRefresh(a => !a)}
            className={`rounded-lg px-2.5 py-1.5 text-xs font-medium transition-all
              ${autoRefresh ? 'action-btn action-btn-success' : 'action-btn action-btn-primary'}
              ${autoRefresh
                ? 'bg-success/10 text-success border border-success/15'
                : 'bg-surface-container text-on-surface-variant border border-outline-variant/60 hover:border-primary/40'
              }`}
          >
            {autoRefresh ? (
              <span className="inline-flex items-center gap-1">
                <i key={heartBeatTick} className="fa-duotone fa-solid fa-heart-pulse heart-beat-once icon-gradient icon-gradient-spring" />
                Live
              </span>
            ) : (
              <span>
                <i className="fa-duotone fa-solid fa-play mr-1 icon-gradient icon-gradient-steel" />
                Paused
              </span>
            )}
          </button>

          <button
            onClick={() => loadFull(false)}
            className="rounded-lg px-2.5 py-1.5 text-xs font-medium text-primary bg-primary/8 hover:bg-primary/14
                       action-btn action-btn-primary
                       transition-colors border border-primary/10"
          >
            <i className="fa-duotone fa-solid fa-arrows-rotate mr-1 icon-gradient icon-gradient-atlas" />Refresh
          </button>

          <button
            onClick={handleClearLogs}
            disabled={clearing}
            className={`rounded-lg px-2.5 py-1.5 text-xs font-medium transition-colors border action-btn action-btn-error
              ${clearing
                ? 'bg-surface-container text-on-surface-variant border-outline-variant/60 opacity-70 cursor-not-allowed'
                : 'text-error bg-error/8 hover:bg-error/14 border-error/20'
              }`}
          >
            <i className={`fa-duotone fa-solid ${clearing ? 'fa-spinner fa-spin text-error' : 'fa-trash-can icon-gradient icon-gradient-ember'} mr-1`} />
            {clearing ? 'Clearing...' : 'Clear'}
          </button>
        </div>
      )}

      <div className="relative rounded-2xl border border-outline-variant/30 bg-surface-container/35 py-2.5 px-1.5 max-h-[calc(100vh-14rem)] overflow-hidden flex flex-col">
        {loading ? (
          <div className="p-3 flex flex-col gap-2">
            <div className="skeleton-line h-3.5 w-96" />
            <div className="skeleton-line h-3.5 w-80" />
            <div className="skeleton-line h-3 w-88" />
            <div className="skeleton-line h-3 w-68" />
          </div>
        ) : lines.length === 0 ? (
          <div className="p-5 flex items-center gap-3">
            <div className="w-9 h-9 rounded-xl bg-surface-container-high/60 border border-outline-variant/25 flex items-center justify-center shadow-elevation-1">
              <i className="fa-duotone fa-solid fa-file-lines icon-gradient icon-gradient-steel icon-sm" />
            </div>
            <div>
              <p className="text-sm text-on-surface-variant">No log entries yet.</p>
              <button
                onClick={() => loadFull(false)}
                className="ui-micro text-primary hover:underline underline-offset-2 mt-1"
              >
                Refresh
              </button>
            </div>
          </div>
        ) : (
          <div ref={scrollRef} className="max-h-[calc(100vh-15rem)] overflow-y-auto scroll-pane" onScroll={handleScroll}>
            <div className="log-viewer">
              <div style={{ height: startIdx * LINE_HEIGHT }} />
              {filteredLines.slice(startIdx, endIdx).map((line, i) => (
                <LogLine key={startIdx + i} line={line} />
              ))}
              <div style={{ height: (totalFiltered - endIdx) * LINE_HEIGHT }} />
            </div>
          </div>
        )}

        {progressBars.length > 0 && (
          <div className="log-progress-footer">
            {progressBars.map((bar, i) => (
              <div key={i} className="log-progress-bar-line">
                <span className="log-tag">[{bar.tag}]</span>{' '}
                {bar.rawBar
                  ? highlightRawBar(bar.rawBar)
                  : bar.current != null && bar.total != null
                    ? renderTqdmBar(bar.current, bar.total, bar.detail, bar.elapsedS)
                    : null
                }
              </div>
            ))}
          </div>
        )}
      </div>

      {!loading && lines.length > 0 && (
        <div className="mt-2 flex justify-end gap-3 px-1">
          <span className="inline-flex items-center gap-1.5 text-xs">
            <span className="w-2 h-2 rounded-full bg-error shadow-[0_0_6px_rgba(239,68,68,0.4)]" />
            <span className="text-error font-semibold tabular-nums">{logStats.errors}</span>
            <span className="text-on-surface-variant">{source === 'test' ? 'failed' : 'errors'}</span>
          </span>
          <span className="inline-flex items-center gap-1.5 text-xs">
            <span className="w-2 h-2 rounded-full bg-warning shadow-[0_0_6px_rgba(245,158,11,0.4)]" />
            <span className="text-warning font-semibold tabular-nums">{logStats.warnings}</span>
            <span className="text-on-surface-variant">warnings</span>
          </span>
          {source === 'test' && (
            <span className="inline-flex items-center gap-1.5 text-xs">
              <span className="w-2 h-2 rounded-full bg-success shadow-[0_0_6px_rgba(34,197,94,0.4)]" />
              <span className="text-success font-semibold tabular-nums">{logStats.passes}</span>
              <span className="text-on-surface-variant">passed</span>
            </span>
          )}
        </div>
      )}
    </div>
  )
}
