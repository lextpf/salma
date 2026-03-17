import { useEffect, useState, useRef, memo, useMemo, useCallback } from 'react'
import { getLogs, getTestLogs, clearLogs } from '../api'

const LINE_OPTIONS = [50, 100, 200, 500, 1000, 0] as const
const RETRY_DELAY_MS = 2000
const LINE_HEIGHT = 26
const OVERSCAN = 20

type LogSource = 'salma' | 'test'

// Pre-pass: extract quoted strings so the main regex doesn't have to deal with them.
// Handles apostrophes inside single-quoted strings (e.g. 'JK's Temple of Talos').
function extractQuoted(text: string): { text: string; cls: string }[] {
  const out: { text: string; cls: string }[] = []
  let i = 0
  let plain = 0

  while (i < text.length) {
    if (text[i] === '"') {
      if (i > plain) out.push({ text: text.slice(plain, i), cls: '' })
      let end = i + 1
      while (end < text.length) {
        if (text[end] === '\\') { end += 2; continue }
        if (text[end] === '"') { end++; break }
        end++
      }
      out.push({ text: text.slice(i, end), cls: 'log-string' })
      i = end; plain = end
    } else if (text[i] === "'") {
      // Closing ' must NOT be followed by an alphanumeric char (that would be an apostrophe)
      let end = i + 1
      let found = false
      while (end < text.length) {
        if (text[end] === "'") {
          const next = end + 1 < text.length ? text[end + 1] : ''
          if (!next || !/[a-zA-Z0-9]/.test(next)) { end++; found = true; break }
        }
        end++
      }
      if (found && end - i > 2) {
        if (i > plain) out.push({ text: text.slice(plain, i), cls: '' })
        out.push({ text: text.slice(i, end), cls: 'log-string' })
        i = end; plain = end
      } else {
        i++
      }
    } else {
      i++
    }
  }
  if (plain < text.length) out.push({ text: text.slice(plain), cls: '' })
  return out
}

const TOKEN_REGEX = new RegExp([
  /(\[[^\]]+\])/,                                          // [tags]
  /(https?:\/\/[^\s,;)"']+)/,                             // URLs
  /([A-Z]:(?:[\\/][^\\/:*?"<>|\r\n,]*[a-zA-Z0-9._()])+|\/(?:usr|var|home|etc|tmp|mnt|opt)\/[^\s,;]+)/, // file paths
  /(HTTP\/\d(?:\.\d)?)/,                                  // HTTP version
  /\b(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)\b/,          // HTTP methods
  /(\/api\/[^\s,;)"']+)/,                                 // API routes
  /\b(\d+\/\d+)\b/,                                        // step counters (0/9, 335/335)
  /\b(PASS|INFERRED|FAIL|SKIP|ERROR|DONE|Passed|Failed)\b/, // test/scan results + summary keywords
  /((?:HTTP(?:\/\d(?:\.\d)?)?\s+[1-5]\d{2}\b)|(?:[Ss][Tt][Aa][Tt][Uu][Ss](?:\s*[Cc][Oo][Dd][Ee])?\s*[:=]\s*[1-5]\d{2}\b)|(?:\b[1-5]\d{2}\b(?=\s+\d+\s*$)))/, // HTTP status
  /(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(?::\d+)?)/,       // IP addresses
  /\b([0-9A-F]{10,})\b/,                                  // hex IDs
  /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i, // GUIDs
  /\b(\d+(?:\.\d+)?(?:ms|s|m|h))\b/,                       // durations
  /\b(\d+(?:\.\d+)?\s*(?:KB|MB|GB|bytes|B|%|files?|folders?|chars|plugins?|groups?|nodes?|steps?|components?))\b/, // numbers with units
  /\b(\w+)(?==(?:\d|true\b|false\b))/,                        // key in key=value pairs (entries=18, exact=true)
  /\b(true|false)\b/,                                        // boolean values
  /\b(\d+(?:\.\d+)?)(?=[kMG]\b|\b)/,                         // plain numbers (stops before SI suffix)
  /(=>|->|::|!=|<=|>=|&&|\|\||=+)/,                          // operators
].map(r => r.source).join('|'), 'g')

function highlightTokens(text: string, parts: { text: string; cls: string }[]) {
  TOKEN_REGEX.lastIndex = 0
  let lastIndex = 0
  let match: RegExpExecArray | null
  while ((match = TOKEN_REGEX.exec(text)) !== null) {
    if (match.index > lastIndex) {
      parts.push({ text: text.slice(lastIndex, match.index), cls: '' })
    }
    const [full, tag, url, path, protocol, method, route, stepCounter, testResult, statusCtx, ip, reqId, guid, duration, unitNum, kvKey, boolVal, num, op] = match
    if (tag) parts.push({ text: full, cls: 'log-tag' })
    else if (url) parts.push({ text: full, cls: 'log-url' })
    else if (path) {
      // Trim trailing non-path text after a file extension (e.g. ".dds (priority: 0)")
      const cleaned = full.replace(/(\.\w{1,10})\s.*$/, '$1')
      if (cleaned.length < full.length) {
        parts.push({ text: cleaned, cls: 'log-path' })
        const saved = TOKEN_REGEX.lastIndex
        highlightTokens(full.slice(cleaned.length), parts)
        TOKEN_REGEX.lastIndex = saved
      } else {
        parts.push({ text: full, cls: 'log-path' })
      }
    }
    else if (protocol) parts.push({ text: full, cls: 'log-protocol' })
    else if (method) parts.push({ text: full, cls: 'log-method' })
    else if (route) parts.push({ text: full, cls: 'log-route' })
    else if (stepCounter) parts.push({ text: full, cls: 'log-number' })
    else if (testResult) {
      const cls = testResult === 'PASS' || testResult === 'Passed' || testResult === 'INFERRED' || testResult === 'DONE' ? 'log-status-ok'
        : testResult === 'FAIL' || testResult === 'ERROR' || testResult === 'Failed' ? 'log-status-err'
        : 'log-level-info' // SKIP
      parts.push({ text: full, cls })
    }
    else if (statusCtx) {
      const code = parseInt((statusCtx.match(/[1-5]\d{2}/) || ['0'])[0], 10)
      parts.push({ text: full, cls: code >= 400 ? 'log-status-err' : code >= 200 && code < 400 ? 'log-status-ok' : 'log-number' })
    }
    else if (ip) parts.push({ text: full, cls: 'log-ip' })
    else if (reqId) parts.push({ text: full, cls: 'log-ip' })
    else if (guid) parts.push({ text: full, cls: 'log-guid' })
    else if (duration) parts.push({ text: full, cls: 'log-duration' })
    else if (unitNum) parts.push({ text: full, cls: 'log-storage' })
    else if (kvKey) parts.push({ text: full, cls: 'log-key' })
    else if (boolVal) parts.push({ text: full, cls: 'log-number' })
    else if (num) parts.push({ text: full, cls: 'log-number' })
    else if (op) parts.push({ text: full, cls: 'log-operator' })
    else parts.push({ text: full, cls: '' })
    lastIndex = match.index + full.length
  }
  if (lastIndex < text.length) {
    parts.push({ text: text.slice(lastIndex), cls: '' })
  }
}

function highlightLog(line: string) {
  const parts: { text: string; cls: string }[] = []
  let remaining = line

  // Match timestamp at start:
  // 2024-01-15 12:34:56(.ms), [2024-01-15 12:34:56], or 3-01 18:37:31.708
  const tsMatch = remaining.match(/^(\[?(?:\d{4}-\d{2}-\d{2}|\d{1,2}-\d{2})[\sT]\d{2}:\d{2}:\d{2}(?:\.\d+)?\]?\s*)/)
  if (tsMatch) {
    parts.push({ text: tsMatch[1], cls: 'log-timestamp' })
    remaining = remaining.slice(tsMatch[1].length)
  }

  // Also match short timestamps like HH:MM:SS(.ms) (test.log format)
  if (!tsMatch) {
    const shortTsMatch = remaining.match(/^(\d{2}:\d{2}:\d{2}(?:\.\d+)?\s+)/)
    if (shortTsMatch) {
      parts.push({ text: shortTsMatch[1], cls: 'log-timestamp' })
      remaining = remaining.slice(shortTsMatch[1].length)
    }
  }

  // Match log level
  const levelMatch = remaining.match(/^(?:-\s*)?(ERROR|WARNING|WARN|INFO|DEBUG|TRACE|CRITICAL|FATAL)\b(?:\s*-(?!-)\s*)?/i)
  if (levelMatch) {
    const level = levelMatch[1].toUpperCase()
    const cls = level === 'ERROR' || level === 'CRITICAL' || level === 'FATAL' ? 'log-level-error'
      : level === 'WARNING' || level === 'WARN' ? 'log-level-warning'
      : level === 'DEBUG' || level === 'TRACE' ? 'log-level-debug'
      : 'log-level-info'
    parts.push({ text: levelMatch[0], cls })
    remaining = remaining.slice(levelMatch[0].length)
  }

  // Pre-pass: extract quoted strings, then highlight unquoted segments with the token regex
  if (remaining) {
    for (const seg of extractQuoted(remaining)) {
      if (seg.cls) parts.push(seg)
      else highlightTokens(seg.text, parts)
    }
  }

  return parts
}

const PROGRESS_BAR_RE = /\d+%\|[=>.]+\|/

function isProgressLine(line: string): boolean {
  return PROGRESS_BAR_RE.test(line)
}

interface TqdmBar {
  tag: string
  rawBar?: string
  current?: number
  total?: number
  detail?: string
  elapsedS?: number
}

function parseLineTimestamp(line: string): number | null {
  const m = line.match(/(?:\d{4}-\d{2}-\d{2}[\sT])?(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?/)
  if (!m) return null
  return parseInt(m[1]) * 3600 + parseInt(m[2]) * 60 + parseInt(m[3])
    + (m[4] ? parseInt(m[4].padEnd(3, '0')) / 1000 : 0)
}

function fmtDur(s: number): string {
  s = Math.floor(s)
  if (s < 60) return `00:${String(s).padStart(2, '0')}`
  if (s < 3600) return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
  return `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
}

function fmtRate(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`
  if (n >= 100) return String(Math.round(n))
  if (n >= 1) return n.toFixed(1)
  return n.toFixed(2)
}

function renderTqdmBar(current: number, total: number, detail?: string, elapsedS?: number, width = 20) {
  if (total <= 0) return null
  const ratio = Math.min(1, current / total)
  const filled = Math.floor(ratio * width)
  const pct = Math.floor(ratio * 100)
  const barFilled = '='.repeat(filled) + (filled < width ? '>' : '')
  const barEmpty = '.'.repeat(Math.max(0, width - filled - 1))

  const showTiming = elapsedS != null && elapsedS > 0 && current > 0
  const rate = showTiming ? current / elapsedS! : 0
  const remainS = showTiming && ratio < 1 ? (total - current) / rate : 0

  return (
    <>
      <span className="text-on-surface font-semibold">{String(pct).padStart(3)}%</span>
      <span className="log-operator">|</span>
      <span className="log-operator">{barFilled}</span>
      <span className="log-operator">{barEmpty}|</span>
      {' '}<span className="log-number">{current}</span>/<span className="log-number">{total}</span>
      {showTiming && (
        <>
          <span className="log-operator">{' ['}</span>
          <span className="log-duration">{fmtDur(elapsedS!)}</span>
          <span className="log-operator">{'<'}</span>
          <span className="log-duration">{fmtDur(remainS)}</span>
          <span className="log-operator">{', '}</span>
          {rate >= 1
            ? <><span className="log-duration">{fmtRate(rate)}</span><span className="log-operator">{'/'}</span>{'s'}</>
            : <><span className="log-duration">{fmtRate(1 / rate)}</span>{'s'}<span className="log-operator">{'/'}</span>{'mod'}</>
          }
          <span className="log-operator">{']'}</span>
        </>
      )}
      {detail && <> {detail}</>}
    </>
  )
}

function highlightRawBar(raw: string) {
  // Format: "  3%|>....................| 1.2k/40k [00:05<02:30, 238/s] | best: m=5 e=3"
  const m = raw.match(/^(\s*\d+%)\|([^|]*)\|\s*(\S+)\/(\S+?)(?:\s+(\w+))?\s*\[([^<]*)<([^,]*),\s*([^/]*)\/s\](.*)$/)
  if (!m) {
    const parts = highlightLog(raw)
    return parts.map((p, j) => p.cls ? <span key={j} className={p.cls}>{p.text}</span> : <span key={j}>{p.text}</span>)
  }
  const [, pct, bar, cur, tot, unit, elapsed, remaining, rate, rest] = m
  return (
    <>
      <span className="text-on-surface font-semibold">{pct}</span>
      <span className="log-operator">|{bar}|</span>
      {' '}{cur.replace(/[kMGT]$/, '').length < cur.length
        ? <><span className="log-number">{cur.slice(0, -1)}</span>{cur.slice(-1)}</>
        : <span className="log-number">{cur}</span>
      }/{tot.replace(/[kMGT]$/, '').length < tot.length
        ? <><span className="log-number">{tot.slice(0, -1)}</span>{tot.slice(-1)}</>
        : <span className="log-number">{tot}</span>
      }{unit && <>{' '}{unit}</>}
      <span className="log-operator">{' ['}</span>
      <span className="log-duration">{elapsed}</span>
      <span className="log-operator">{'<'}</span>
      <span className="log-duration">{remaining}</span>
      <span className="log-operator">{', '}</span>
      <span className="log-duration">{rate}</span>
      <span className="log-operator">{'/'}</span>{'s'}<span className="log-operator">{']'}</span>
      {rest && highlightLog(rest).map((p, j) =>
        p.cls ? <span key={`r${j}`} className={p.cls}>{p.text}</span> : <span key={`r${j}`}>{p.text}</span>
      )}
    </>
  )
}

function parseProgressBars(lines: string[], source: LogSource, cachedScanBar?: React.MutableRefObject<TqdmBar | null>, cachedStartTsRef?: React.MutableRefObject<number | null>): TqdmBar[] {
  const bars: TqdmBar[] = []

  if (source === 'salma') {
    let solverRaw: string | null = null
    let solverDone = false

    for (let i = lines.length - 1; i >= 0 && i >= lines.length - 300; i--) {
      const line = lines[i]
      if (!solverDone && /\[solver\] Done:|\[solver\] No solution found/.test(line)) {
        solverDone = true
        continue // keep scanning backward for the completion bar
      }
      if (!solverRaw) {
        const m = line.match(/\[solver\]\s+(\d+%\|.+)/)
        if (m) { solverRaw = m[1]; break }
      }
    }

    if (solverRaw) bars.push({ tag: 'solver', rawBar: solverRaw })

    // Scan progress bar (same [N/M] format as test, but in salma.log under [infer])
    let scanBar: TqdmBar | null = null
    let scanDone = false
    let scanLatestTs: number | null = null
    let scanStartTs: number | null = null

    for (let i = lines.length - 1; i >= 0 && i >= lines.length - 500; i--) {
      const line = lines[i]

      if (/\[infer\] Scan complete:/.test(line)) { scanDone = true; break }

      if (!scanBar) {
        const m = line.match(/\[infer\]\s+\[(\d+)\/(\d+)\]\s+(.+?)\.{3,}\s+(?:PASS|FAIL|SKIP|ERROR|INFERRED|NOT FOMOD|NO STEPS)/)
        if (m) {
          scanBar = { tag: 'scan', current: parseInt(m[1]), total: parseInt(m[2]), detail: m[3].trim() }
          scanLatestTs = parseLineTimestamp(line)
        }
      }

      if (scanBar && scanStartTs == null && /\[infer\]\s+\[1\/\d+\]/.test(line)) {
        scanStartTs = parseLineTimestamp(line)
      }

      if (scanBar && scanStartTs != null) break
    }

    if (scanBar && !scanDone && scanStartTs == null) {
      for (let j = 0; j < lines.length && j < 200; j++) {
        if (/\[infer\]\s+\[1\/\d+\]/.test(lines[j])) {
          scanStartTs = parseLineTimestamp(lines[j])
          break
        }
      }
    }

    // Cache start timestamp so it survives after [1/N] scrolls out of the line buffer
    if (scanStartTs != null && cachedStartTsRef) {
      cachedStartTsRef.current = scanStartTs
    } else if (scanStartTs == null && cachedStartTsRef?.current != null) {
      scanStartTs = cachedStartTsRef.current
    }

    if (scanBar && !scanDone) {
      if (scanLatestTs != null && scanStartTs != null) scanBar.elapsedS = scanLatestTs - scanStartTs
      if (cachedScanBar) cachedScanBar.current = scanBar
      bars.push(scanBar)
    } else if (!scanBar && !scanDone && cachedScanBar?.current) {
      // Sticky: show cached bar while scan is in progress but no match in current window
      bars.push(cachedScanBar.current)
    } else if (scanDone && cachedScanBar) {
      cachedScanBar.current = null
    }
    if (scanDone && cachedStartTsRef) cachedStartTsRef.current = null

  } else {
    let testBar: TqdmBar | null = null
    let testDone = false
    let latestTs: number | null = null
    let startTs: number | null = null

    for (let i = lines.length - 1; i >= 0 && i >= lines.length - 500; i--) {
      const line = lines[i]

      if (/Tested:\s*\d+\s+Passed:/.test(line)) { testDone = true; break }

      if (!testBar) {
        const m = line.match(/\[(\d+)\/(\d+)\]\s+(.+?)\.{3,}\s+(?:PASS|FAIL|SKIP|ERROR|INFERRED)/)
        if (m) {
          testBar = { tag: 'test', current: parseInt(m[1]), total: parseInt(m[2]), detail: m[3].trim() }
          latestTs = parseLineTimestamp(line)
        }
      }

      if (testBar && startTs == null && /\[1\/\d+\]/.test(line)) {
        startTs = parseLineTimestamp(line)
      }

      if (testBar && startTs != null) break
    }

    // If the [1/N] line fell outside the backwards window, scan forward
    if (testBar && !testDone && startTs == null) {
      for (let j = 0; j < lines.length && j < 200; j++) {
        if (/\[1\/\d+\]/.test(lines[j])) {
          startTs = parseLineTimestamp(lines[j])
          break
        }
      }
    }

    // Cache start timestamp so it survives after [1/N] scrolls out of the line buffer
    if (startTs != null && cachedStartTsRef) {
      cachedStartTsRef.current = startTs
    } else if (startTs == null && cachedStartTsRef?.current != null) {
      startTs = cachedStartTsRef.current
    }

    if (testBar && !testDone) {
      if (latestTs != null && startTs != null) testBar.elapsedS = latestTs - startTs
      bars.push(testBar)
    }
    if (testDone && cachedStartTsRef) cachedStartTsRef.current = null
  }

  return bars
}

const LogLine = memo(function LogLine({ line }: { line: string }) {
  const isError = /\bERROR\b|\bCRITICAL\b|\bFATAL\b|\bFAIL\b/i.test(line)
  const isWarning = /\bWARNING\b|\bWARN\b/i.test(line)
  const isPass = /\bPASS\b|\bINFERRED\b/.test(line)
  const parts = highlightLog(line)
  return (
    <div
      className={`py-0.5 px-2 rounded ${
        isError ? 'bg-error/5' :
        isWarning ? 'bg-warning/5' :
        isPass ? 'bg-success/5' :
        ''
      }`}
    >
      {parts.map((p, j) => (
        p.cls
          ? <span key={j} className={p.cls}>{p.text}</span>
          : <span key={j}>{p.text}</span>
      ))}
    </div>
  )
})

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
  const scrollRef = useRef<HTMLDivElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)
  const offsetRef = useRef<number | undefined>(undefined)
  const statsRef = useRef({ errors: 0, warnings: 0, passes: 0 })
  const cachedScanBarRef = useRef<TqdmBar | null>(null)
  const cachedScanStartTsRef = useRef<number | null>(null)
  const cachedTestStartTsRef = useRef<number | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const isAtBottomRef = useRef(true)
  const rafRef = useRef(0)

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
  const loadFull = (showBusy = false, force = false) => {
    if (inFlightRef.current && !force) return
    if (showBusy) setLoading(true)
    inFlightRef.current = true
    const fetcher = source === 'test' ? getTestLogs : getLogs
    fetcher(lineCount)
      .then(data => {
        setLines(data.lines)
        const stats = { errors: data.errors ?? 0, warnings: data.warnings ?? 0, passes: data.passes ?? 0 }
        statsRef.current = stats
        setLogStats(stats)
        offsetRef.current = data.nextOffset
        setLoading(false)
        clearRetryTimer()
        if (autoRefresh) {
          setHeartBeatTick(t => t + 1)
        }
      })
      .catch(e => {
        console.warn(`[logs] failed to load ${source}.log, retrying`, e)
        setLoading(true)
        scheduleRetry()
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }

  // Incremental load: fetches only new lines since last offset
  const loadIncremental = () => {
    if (inFlightRef.current) return
    if (offsetRef.current == null) { loadFull(); return }
    inFlightRef.current = true
    const fetcher = source === 'test' ? getTestLogs : getLogs
    fetcher(lineCount, offsetRef.current)
      .then(data => {
        if (data.reset) {
          // Log was cleared - do a full reload
          offsetRef.current = undefined
          statsRef.current = { errors: 0, warnings: 0, passes: 0 }
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
            if (lineCount > 0 && combined.length > lineCount) {
              return combined.slice(combined.length - lineCount)
            }
            return combined
          })
          const s = statsRef.current
          s.errors += data.errors ?? 0
          s.warnings += data.warnings ?? 0
          s.passes += data.passes ?? 0
          setLogStats({ ...s })
        }
        clearRetryTimer()
        if (autoRefresh) {
          setHeartBeatTick(t => t + 1)
        }
      })
      .catch(e => {
        console.warn(`[logs] incremental load failed`, e)
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }

  // Reset offset when source or lineCount changes
  useEffect(() => {
    offsetRef.current = undefined
    statsRef.current = { errors: 0, warnings: 0, passes: 0 }
    cachedScanBarRef.current = null
    isAtBottomRef.current = true
    setScrollTop(0)
    loadFull(true)
  }, [lineCount, source])

  useEffect(() => {
    if (!autoRefresh) return
    const id = setInterval(loadIncremental, 1000)
    return () => clearInterval(id)
  }, [autoRefresh, lineCount, source])

  useEffect(() => {
    if (!isAtBottomRef.current) return
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [lines])

  useEffect(() => clearRetryTimer, [])
  useEffect(() => () => { if (rafRef.current) cancelAnimationFrame(rafRef.current) }, [])

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

  const handleScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    if (rafRef.current) cancelAnimationFrame(rafRef.current)
    rafRef.current = requestAnimationFrame(() => {
      setScrollTop(el.scrollTop)
      isAtBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < LINE_HEIGHT
    })
  }, [])

  const handleClearLogs = async () => {
    setClearing(true)
    try {
      await clearLogs(source)
      offsetRef.current = undefined
      statsRef.current = { errors: 0, warnings: 0, passes: 0 }
      cachedScanBarRef.current = null
      cachedScanStartTsRef.current = null
      cachedTestStartTsRef.current = null
      isAtBottomRef.current = true
      setScrollTop(0)
      loadFull()
    } catch (e) {
      console.warn(`[logs] failed to clear ${source}.log`, e)
    } finally {
      setClearing(false)
    }
  }

  const totalFiltered = filteredLines.length
  const containerHeight = scrollRef.current?.clientHeight ?? 600
  const startIdx = Math.max(0, Math.floor(scrollTop / LINE_HEIGHT) - OVERSCAN)
  const endIdx = Math.min(totalFiltered, Math.floor(scrollTop / LINE_HEIGHT) + Math.ceil(containerHeight / LINE_HEIGHT) + OVERSCAN)

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

        {(() => {
          const startTsRef = source === 'salma' ? cachedScanStartTsRef : cachedTestStartTsRef
          const bars = !loading && lines.length > 0 ? parseProgressBars(lines, source, cachedScanBarRef, startTsRef) : []
          if (bars.length === 0) return null
          return (
            <div className="log-progress-footer">
              {bars.map((bar, i) => (
                <div key={i} className="log-progress-bar-line">
                  <span className="log-tag">[{bar.tag}]</span>{' '}
                  {bar.rawBar
                    ? highlightRawBar(bar.rawBar)
                    : renderTqdmBar(bar.current!, bar.total!, bar.detail, bar.elapsedS)
                  }
                </div>
              ))}
            </div>
          )
        })()}
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
