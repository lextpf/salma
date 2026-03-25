import React from 'react'
import { highlightLog } from './logHighlight'

const PROGRESS_BAR_RE = /\d+%\|[=>.]+\|/

export function isProgressLine(line: string): boolean {
  return PROGRESS_BAR_RE.test(line)
}

export interface TqdmBar {
  tag: string
  rawBar?: string
  current?: number
  total?: number
  detail?: string
  elapsedS?: number
}

export function parseLineTimestamp(line: string): number | null {
  const m = line.match(/(?:\d{4}-\d{2}-\d{2}[\sT])?(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?/)
  if (!m) return null
  return parseInt(m[1]) * 3600 + parseInt(m[2]) * 60 + parseInt(m[3])
    + (m[4] ? parseInt(m[4].padEnd(3, '0')) / 1000 : 0)
}

export function fmtDur(s: number): string {
  s = Math.floor(s)
  if (s < 60) return `00:${String(s).padStart(2, '0')}`
  if (s < 3600) return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
  return `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
}

export function fmtRate(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`
  if (n >= 100) return String(Math.round(n))
  if (n >= 1) return n.toFixed(1)
  return n.toFixed(2)
}

export function renderTqdmBar(current: number, total: number, detail?: string, elapsedS?: number, width = 20) {
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

export function highlightRawBar(raw: string) {
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

type LogSource = 'salma' | 'test'

export function parseProgressBars(lines: string[], source: LogSource, cachedScanBar?: React.MutableRefObject<TqdmBar | null>, cachedStartTsRef?: React.MutableRefObject<number | null>): TqdmBar[] {
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
