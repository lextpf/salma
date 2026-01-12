import { useEffect, useState, useRef, useMemo, useCallback } from 'react'
import { getLogs, getTestLogs, clearLogs } from '../api'
import { isProgressLine, parseProgressBars, highlightRawBar, renderTqdmBar, TqdmBar } from '../utils/progressBarParsing'
import { LogLine } from '../components/LogLine'
import Section from '../components/Section'
import { useVirtualScroll, LINE_HEIGHT } from '../hooks/useVirtualScroll'

const LINE_OPTIONS = [50, 100, 200, 500, 1000, 0] as const
const RETRY_DELAY_MS = 2000

type LogSource = 'salma' | 'test'

export default function LogsPage() {
  const [lines, setLines] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [autoRefresh, setAutoRefresh] = useState(true)
  const [lineCount, setLineCount] = useState(1000)
  const [dropdownOpen, setDropdownOpen] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [source, setSource] = useState<LogSource>('salma')
  const [logStats, setLogStats] = useState({ errors: 0, warnings: 0, passes: 0 })
  const { scrollRef, scrollEl, handleScroll, isAtBottomRef, resetScroll, startIdx: getStartIdx, endIdx: getEndIdx } = useVirtualScroll()
  const dropdownRef = useRef<HTMLDivElement>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const refreshBusyRef = useRef(false)
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
      loadFull(false)
    }, RETRY_DELAY_MS)
  }

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

  const loadFull = useCallback((showBusy = false) => {
    if (showBusy) setLoading(true)
    refreshBusyRef.current = true
    const currentSource = sourceRef.current
    const currentLineCount = lineCountRef.current
    const fetcher = currentSource === 'test' ? getTestLogs : getLogs
    return fetcher(currentLineCount)
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
    const currentLineCount = lineCountRef.current
    const fetcher = currentSource === 'test' ? getTestLogs : getLogs
    return fetcher(currentLineCount, offsetRef.current)
      .then(data => {
        if (abortRef.current?.signal.aborted) return
        if (data.reset) {
          offsetRef.current = undefined
          setLogStats({ errors: 0, warnings: 0, passes: 0 })
          cachedScanBarRef.current = null
          cachedScanStartTsRef.current = null
          cachedTestStartTsRef.current = null
          return loadFull()
        }
        offsetRef.current = data.nextOffset
        if (data.lines.length > 0) {
          setLines(prev => {
            const combined = [...prev, ...data.lines]
            if (currentLineCount > 0 && combined.length > currentLineCount) {
              return combined.slice(combined.length - currentLineCount)
            }
            return combined
          })
          setLogStats(prev => ({
            errors:   prev.errors   + (data.errors ?? 0),
            warnings: prev.warnings + (data.warnings ?? 0),
            passes:   prev.passes   + (data.passes ?? 0),
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
    cachedScanBarRef.current = null
    resetScroll()
    loadFull(true)
    return () => {
      if (abortRef.current) abortRef.current.abort()
    }
  }, [lineCount, source, loadFull, resetScroll])

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
    return () => { active = false; clearTimeout(tid) }
  }, [autoRefresh, loadIncremental])

  useEffect(() => {
    if (!isAtBottomRef.current) return
    const el = scrollEl.current
    if (el) el.scrollTop = el.scrollHeight
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lines, scrollEl])

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

  const sectionTitle = source === 'salma' ? 'salma.log' : 'test.log'

  return (
    <div className="page-fill">
      <header style={{ marginBottom: 20, flexShrink: 0 }}>
        <p
          className="reveal reveal-delay-1"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 10,
            fontFamily: 'var(--font-mono)',
            fontSize: 11,
            letterSpacing: '0.18em',
            textTransform: 'uppercase',
            color: 'var(--ink-3)',
            marginBottom: 10,
          }}
        >
          <span
            className="display-serif-italic"
            style={{ fontSize: 12, color: 'var(--accent)', textTransform: 'none' }}
          >
            03.
          </span>
          <span>Sec. Stream</span>
        </p>

        <div className="flex items-end reveal reveal-delay-2" style={{ gap: 20, flexWrap: 'wrap' }}>
          <h1
            className="display-serif"
            style={{ fontSize: 72, lineHeight: 0.92, color: 'var(--ink)', margin: 0 }}
          >
            Logs<span style={{ color: 'var(--accent)' }}>.</span>
          </h1>
          {!loading && (
            <span style={{ marginBottom: 10 }}>
              <span
                className="display-serif tabular-nums"
                style={{ fontSize: 24, color: 'var(--ink)', letterSpacing: '-0.02em' }}
              >
                {lines.length}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 11,
                  color: 'var(--ink-4)',
                  marginLeft: 6,
                  letterSpacing: '0.04em',
                }}
              >
                lines
              </span>
            </span>
          )}
        </div>
        <p
          className="reveal reveal-delay-3 display-serif-italic"
          style={{
            margin: '14px 0 0',
            fontSize: 14,
            color: 'var(--ink-3)',
            maxWidth: 620,
            lineHeight: 1.5,
          }}
        >
          Tailing the live{' '}
          <SourceSwitch active={source === 'salma'} onClick={() => setSource('salma')}>
            salma.log
          </SourceSwitch>
          <span style={{ margin: '0 4px', color: 'var(--ink-5)' }}>-</span>
          <SourceSwitch active={source === 'test'} onClick={() => setSource('test')}>
            test.log
          </SourceSwitch>{' '}
          stream - pause to inspect.
        </p>
      </header>

      <Section
        n="01"
        label="Stream"
        title={sectionTitle}
        corner="01"
        bodyPadding="none"
        className={`reveal reveal-delay-4${loading ? '' : ' atelier-section-fill'}`}
        meta={
            <div className="flex items-center" style={{ gap: 8, flexWrap: 'wrap' }}>
              {/* Line count dropdown */}
              <div style={{ position: 'relative' }} ref={dropdownRef}>
                <button
                  type="button"
                  onClick={() => setDropdownOpen(o => !o)}
                  className="tool-btn"
                  style={{ padding: '6px 10px', fontSize: 11 }}
                >
                  <span className="tabular-nums" style={{ color: 'var(--ink-2)' }}>
                    {lineCount === 0 ? 'all' : lineCount}
                  </span>
                  <span style={{ color: 'var(--ink-4)', fontFamily: 'var(--font-mono)' }}>lines</span>
                  <i
                    className={`fa-duotone fa-solid fa-angle-down ${dropdownOpen ? 'fa-rotate-180' : ''}`}
                    style={{ fontSize: 9 }}
                  />
                </button>
                {dropdownOpen && (
                  <div
                    style={{
                      position: 'absolute',
                      right: 0,
                      top: 'calc(100% + 4px)',
                      minWidth: 100,
                      padding: '4px 0',
                      background: 'var(--card)',
                      border: '1px solid var(--rule)',
                      borderRadius: 'var(--radius-sm)',
                      boxShadow: 'var(--shadow-elevation-2)',
                      zIndex: 50,
                    }}
                  >
                    {LINE_OPTIONS.map(n => (
                      <button
                        key={n}
                        type="button"
                        onClick={() => { setLineCount(n); setDropdownOpen(false) }}
                        style={{
                          width: '100%',
                          padding: '6px 12px',
                          fontSize: 11,
                          fontFamily: 'var(--font-mono)',
                          background: n === lineCount ? 'var(--paper-3)' : 'transparent',
                          color: n === lineCount ? 'var(--accent)' : 'var(--ink-2)',
                          border: 'none',
                          cursor: 'pointer',
                          textAlign: 'left',
                          letterSpacing: '0.04em',
                        }}
                      >
                        {n === 0 ? 'all' : n}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <button
                type="button"
                onClick={() => setAutoRefresh(a => !a)}
                className={`tool-btn ${autoRefresh ? '' : ''}`}
                style={{ padding: '6px 10px', fontSize: 11 }}
              >
                <span
                  className={autoRefresh ? 'dot-status dot-status-on dot-status-pulse' : 'dot-status dot-status-off'}
                />
                <span style={{ fontFamily: 'var(--font-mono)', letterSpacing: '0.08em', color: 'var(--ink-3)', textTransform: 'uppercase' }}>
                  {autoRefresh ? 'Live' : 'Paused'}
                </span>
              </button>

              <button
                type="button"
                onClick={() => { if (!refreshBusyRef.current) loadFull(false) }}
                className="tool-btn"
                style={{ padding: '6px 10px', fontSize: 11 }}
                title="Refresh"
              >
                <i className="fa-duotone fa-solid fa-arrows-rotate" style={{ fontSize: 11 }} />
              </button>

              <button
                type="button"
                onClick={handleClearLogs}
                disabled={clearing}
                className="tool-btn"
                style={{ padding: '6px 10px', fontSize: 11 }}
                title="Clear log"
              >
                <i
                  className={`fa-duotone fa-solid fa-broom${clearing ? ' fa-shake' : ''}`}
                  style={{ fontSize: 11 }}
                />
              </button>

              {/* Stat dots */}
              <span style={{ width: 1, height: 14, background: 'var(--rule)', margin: '0 4px' }} />
              <Stat color="var(--accent)" value={logStats.errors} label={source === 'test' ? 'failed' : 'errors'} />
              <Stat color="var(--ochre)"  value={logStats.warnings} label="warn" />
              {source === 'test' && (
                <Stat color="var(--moss)" value={logStats.passes} label="pass" />
              )}
            </div>
          }
        >
          <div
            style={{
              borderTop: '1px solid var(--rule-soft)',
              background: 'var(--card)',
              flex: 1,
              minHeight: 0,
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            {loading ? (
              <div style={{ padding: '8px 0' }}>
                <div className="log-viewer" style={{ padding: '0 20px' }}>
                  {[72, 58, 81, 49, 68, 55].map((w, i) => (
                    <div
                      key={i}
                      style={{ height: LINE_HEIGHT, display: 'flex', alignItems: 'center' }}
                    >
                      <div className="skeleton-line" style={{ height: 11, width: `${w}%` }} />
                    </div>
                  ))}
                </div>
              </div>
            ) : lines.length === 0 ? (
              <div style={{ padding: 28 }}>
                <p
                  className="display-serif-italic"
                  style={{ fontSize: 18, color: 'var(--ink)', lineHeight: 1.2 }}
                >
                  No log entries yet<span className="display-period">.</span>
                </p>
                <button
                  onClick={() => loadFull(false)}
                  type="button"
                  className="timestamp-print"
                  style={{
                    marginTop: 6,
                    background: 'transparent',
                    border: 'none',
                    color: 'var(--accent)',
                    cursor: 'pointer',
                    padding: 0,
                    textDecoration: 'underline',
                    textUnderlineOffset: 2,
                  }}
                >
                  // refresh
                </button>
              </div>
            ) : (
              <div
                ref={scrollRef}
                onScroll={handleScroll}
                className="scroll-pane"
                style={{
                  flex: 1,
                  overflow: 'auto',
                  padding: '8px 0',
                }}
              >
                <div className="log-viewer" style={{ padding: '0 20px' }}>
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
      </Section>
    </div>
  )
}

function SourceSwitch({
  active,
  onClick,
  children,
}: {
  active: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <span
      role="button"
      tabIndex={0}
      onClick={onClick}
      onKeyDown={e => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick() }
      }}
      style={{
        cursor: active ? 'default' : 'pointer',
        fontStyle: 'normal',
        fontFamily: 'var(--font-body)',
        fontSize: 13,
        fontWeight: active ? 700 : 400,
        color: active ? 'var(--ink)' : 'var(--ink-4)',
        transition: 'color 150ms ease',
      }}
      onMouseEnter={e => { if (!active) e.currentTarget.style.color = 'var(--ink-2)' }}
      onMouseLeave={e => { if (!active) e.currentTarget.style.color = 'var(--ink-4)' }}
    >
      {children}
    </span>
  )
}

function Stat({ color, value, label }: { color: string; value: number; label: string }) {
  return (
    <span className="flex items-center" style={{ gap: 5 }}>
      <span style={{ width: 5, height: 5, borderRadius: '50%', background: color }} />
      <span
        className="tabular-nums"
        style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--ink-2)' }}
      >
        {value}
      </span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          color: 'var(--ink-4)',
          letterSpacing: '0.06em',
          textTransform: 'uppercase',
        }}
      >
        {label}
      </span>
    </span>
  )
}
