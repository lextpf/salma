import React from 'react'
import { highlightLog } from './logHighlight'

/** A rendered tqdm bar anywhere in the line, such as ` 42%|===>......|`. */
const PROGRESS_BAR_RE = /\d+%\|[=>.]+\|/

/**
 * True for a line whose content is a progress bar.
 *
 * The Logs stream drops these from the record list and the docked footer shows
 * them as real bars instead, so a line counted here must not also be parsed as
 * a log record.
 */
export function isProgressLine(line: string): boolean {
  return PROGRESS_BAR_RE.test(line)
}

export interface TqdmBar {
  /** Producer: 'solver', 'scan' or 'test'. Rendered as the phase label. */
  tag: string
  /** The solver's tqdm line, carried through unparsed. Absent for scan and test. */
  rawBar?: string
  current?: number
  total?: number
  /** The item being worked on, for scan and test bars. */
  detail?: string
  /** Seconds from the [1/N] line to the newest matched line. */
  elapsedS?: number
}

/**
 * Seconds since midnight for a line's timestamp, or null when it has none.
 *
 * The date part is matched but discarded, so this is a time of day and not an
 * instant: a run that crosses midnight produces a negative elapsed time.
 */
export function parseLineTimestamp(line: string): number | null {
  const m = line.match(/(?:\d{4}-\d{2}-\d{2}[\sT])?(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?/)
  if (!m) return null
  return parseInt(m[1]) * 3600 + parseInt(m[2]) * 60 + parseInt(m[3])
    + (m[4] ? parseInt(m[4].padEnd(3, '0')) / 1000 : 0)
}

/** Whole seconds as MM:SS, or H:MM:SS from an hour up. */
export function fmtDur(s: number): string {
  s = Math.floor(s)
  if (s < 60) return `00:${String(s).padStart(2, '0')}`
  if (s < 3600) return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
  return `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
}

/** Compact rate: 1.2M, 40k, 238, 4.2, 0.05. Never wider than five characters. */
export function fmtRate(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`
  if (n >= 100) return String(Math.round(n))
  if (n >= 1) return n.toFixed(1)
  return n.toFixed(2)
}

/**
 * Draw a bar from counts, in the tqdm shape the log itself writes.
 *
 * `width` is in characters, not pixels. Returns null when `total` is zero or
 * negative; the caller has to fall back to an indeterminate fill. The timing
 * block appears only with a positive `elapsedS` and a positive `current`, and
 * below one item per second the rate is inverted to seconds per mod.
 */
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
      <span style={{ color: 'var(--ink-2)', fontWeight: 600 }}>{String(pct).padStart(3)}%</span>
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

/**
 * Colour a raw tqdm line from the log. Falls back to highlightLog when the line
 * does not match the expected shape:
 *
 *   "  3%|>....................| 1.2k/40k [00:05<02:30, 238/s] | best: m=5 e=3"
 */
export function highlightRawBar(raw: string) {
  const m = raw.match(/^(\s*\d+%)\|([^|]*)\|\s*(\S+)\/(\S+?)(?:\s+(\w+))?\s*\[([^<]*)<([^,]*),\s*([^/]*)\/s\](.*)$/)
  if (!m) {
    const parts = highlightLog(raw)
    return parts.map((p, j) => p.cls ? <span key={j} className={p.cls}>{p.text}</span> : <span key={j}>{p.text}</span>)
  }
  const [, pct, bar, cur, tot, unit, elapsed, remaining, rate, rest] = m
  return (
    <>
      <span style={{ color: 'var(--ink-2)', fontWeight: 600 }}>{pct}</span>
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

/**
 * Recover the live progress bars from a window of log lines.
 *
 * Two producers write progress, in two shapes:
 *   - the solver emits a ready-made tqdm bar under [solver], kept as `rawBar`
 *   - the scan ([infer]) and the test harness emit "[N/M] name... <result>"
 *     lines, from which `current`, `total` and `detail` are recovered
 *
 * The search runs backwards from the tail: 300 lines for the solver, 500 for
 * the scan and the test bar. A scan or test bar is dropped once its completion
 * marker is inside that window, so a finished run shows nothing.
 *
 * Two pieces of state have to outlive the window and are passed in as refs by
 * the caller: the last matched scan bar, and the timestamp of the [1/N] line
 * elapsed time is measured from. Without them the readout blanks out as soon as
 * those lines scroll off. Both refs are written here, which is why this runs on
 * arrival in applyLines and never inside a render.
 */
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

    // Cached so elapsed time survives [1/N] scrolling out of the buffer.
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
      // The scan is still running but no [N/M] line is in the window; keep
      // showing the last one rather than blanking the footer.
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

    // The [1/N] line fell outside the backwards window; look from the front.
    if (testBar && !testDone && startTs == null) {
      for (let j = 0; j < lines.length && j < 200; j++) {
        if (/\[1\/\d+\]/.test(lines[j])) {
          startTs = parseLineTimestamp(lines[j])
          break
        }
      }
    }

    // Cached so elapsed time survives [1/N] scrolling out of the buffer.
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
