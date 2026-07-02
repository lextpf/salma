export type LogLevel = 'INFO' | 'DEBUG' | 'WARN' | 'ERROR' | ''

export interface LogRecord {
  /**
   * @brief time of day without a date.
   */
  ts?: string
  level: LogLevel
  /**
   * @brief first valid subsystem tag without brackets.
   */
  subsystem: string
  message: string
  raw: string
}

// keep timestamp and level recognition synchronized with `logHighlight.ts`.
const TS_FULL_RE =
  /^(\[?(?:\d{4}-\d{2}-\d{2}|\d{1,2}-\d{2})[\sT]\d{2}:\d{2}:\d{2}(?:\.\d+)?\]?\s*)/
const TS_SHORT_RE = /^(\d{2}:\d{2}:\d{2}(?:\.\d+)?\s+)/
const LEVEL_RE =
  /^(?:-\s*)?(ERROR|WARNING|WARN|INFO|DEBUG|TRACE|CRITICAL|FATAL)\b(?:\s*-(?!-)\s*)?/i
const TAG_RE = /\[([^\]]+)\]/
// reject progress counters and interleaved records that resemble subsystem tags.
const SUBSYSTEM_RE = /^[A-Za-z][\w.-]{0,23}$/
const TOD_RE = /(\d{1,2}):(\d{2}):(\d{2})(?:\.(\d+))?/

export function normalizeLevel(raw: string): LogLevel {
  switch (raw.toUpperCase()) {
    case 'ERROR':
    case 'CRITICAL':
    case 'FATAL':
      return 'ERROR'
    case 'WARN':
    case 'WARNING':
      return 'WARN'
    case 'DEBUG':
    case 'TRACE':
      return 'DEBUG'
    case 'INFO':
      return 'INFO'
    default:
      return ''
  }
}

export function parseLogLine(line: string): LogRecord {
  let remaining = line
  let ts: string | undefined

  const tsMatch = remaining.match(TS_FULL_RE) ?? remaining.match(TS_SHORT_RE)
  if (tsMatch) {
    const tod = tsMatch[1].match(TOD_RE)
    ts = tod ? tod[0] : tsMatch[1].trim()
    remaining = remaining.slice(tsMatch[1].length)
  }

  let level: LogLevel = ''
  const levelMatch = remaining.match(LEVEL_RE)
  if (levelMatch) {
    level = normalizeLevel(levelMatch[1])
    remaining = remaining.slice(levelMatch[0].length)
  }

  let subsystem = ''
  let message = remaining.trim()
  const tagMatch = remaining.match(TAG_RE)
  if (tagMatch && tagMatch.index != null && SUBSYSTEM_RE.test(tagMatch[1].trim())) {
    subsystem = tagMatch[1].trim()
    message = (remaining.slice(0, tagMatch.index) + remaining.slice(tagMatch.index + tagMatch[0].length)).trim()
  }

  return { ts, level, subsystem, message, raw: line }
}

/**
 * @fn parseTimeMs(ts?: string): number | null
 * @brief locate a log record within one day.
 * @author Alex (https://github.com/lextpf)
 *
 * dates are discarded.
 * @return milliseconds since midnight, or null without a recognized time.
 */
export function parseTimeMs(ts?: string): number | null {
  if (!ts) {
    return null
  }
  const m = ts.match(TOD_RE)
  if (!m) {
    return null
  }
  const h = Number(m[1])
  const min = Number(m[2])
  const s = Number(m[3])
  const frac = m[4] ? Number(`0.${m[4]}`) : 0
  return ((h * 60 + min) * 60 + s) * 1000 + Math.round(frac * 1000)
}

export interface HistogramBucket {
  count: number
  warn: number
  error: number
}

/**
 * @fn buildHistogram(records: LogRecord[], buckets = 12): HistogramBucket[]
 * @brief distribute log records by time with an order-based fallback.
 * @author Alex (https://github.com/lextpf)
 *
 * record order is used when timestamps cannot define a positive span. time-only
 * values can order a run across midnight incorrectly.
 */
export function buildHistogram(records: LogRecord[], buckets = 12): HistogramBucket[] {
  const out: HistogramBucket[] = Array.from({ length: buckets }, () => ({
    count: 0,
    warn: 0,
    error: 0,
  }))
  const n = records.length
  if (n === 0) {
    return out
  }

  const times = records.map((r) => parseTimeMs(r.ts))
  let min = Number.POSITIVE_INFINITY
  let max = Number.NEGATIVE_INFINITY
  let validCount = 0
  for (const t of times) {
    if (t == null) {
      continue
    }
    validCount += 1
    if (t < min) {
      min = t
    }
    if (t > max) {
      max = t
    }
  }
  const useTime = validCount > 1 && max > min

  records.forEach((r, i) => {
    let b: number
    if (useTime) {
      const t = times[i]
      b = t == null ? buckets - 1 : Math.min(buckets - 1, Math.floor(((t - min) / (max - min)) * buckets))
    } else {
      b = Math.min(buckets - 1, Math.floor((i / n) * buckets))
    }
    const bucket = out[b]
    bucket.count += 1
    if (r.level === 'ERROR') {
      bucket.error += 1
    } else if (r.level === 'WARN') {
      bucket.warn += 1
    }
  })

  return out
}

export interface SubsystemFacet {
  tag: string
  count: number
}

export function facetCounts(records: LogRecord[]): SubsystemFacet[] {
  const counts = new Map<string, number>()
  for (const r of records) {
    if (!r.subsystem) {
      continue
    }
    counts.set(r.subsystem, (counts.get(r.subsystem) ?? 0) + 1)
  }
  return [...counts.entries()]
    .map(([tag, count]) => ({ tag, count }))
    .sort((a, b) => b.count - a.count || a.tag.localeCompare(b.tag))
}
