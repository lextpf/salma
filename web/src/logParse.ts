// Pure parsing/aggregation helpers for the Logs screen. Kept in a .ts file
// (no JSX) so the .tsx components stay component-only. Detection mirrors
// logHighlight.ts: the same timestamp shapes, the same level keywords, and the
// first [TAG] as the subsystem.

export type LogLevel = 'INFO' | 'DEBUG' | 'WARN' | 'ERROR' | ''

export interface LogRecord {
  // Time-of-day portion of the timestamp, e.g. "11:53:10.482" (date stripped
  // for display). Undefined when the line has no recognizable timestamp.
  ts?: string
  level: LogLevel
  // First [TAG] content with the brackets removed, e.g. "propagate". '' if none.
  subsystem: string
  // Everything after timestamp/level/leading tag, fed to highlightLog.
  message: string
  // Original line, used for the free-text filter.
  raw: string
}

// Long timestamp at line start: 2024-01-15 12:34:56(.ms), [..], or 3-01 18:37:31.708
const TS_FULL_RE =
  /^(\[?(?:\d{4}-\d{2}-\d{2}|\d{1,2}-\d{2})[\sT]\d{2}:\d{2}:\d{2}(?:\.\d+)?\]?\s*)/
// Short timestamp (test.log): HH:MM:SS(.ms)
const TS_SHORT_RE = /^(\d{2}:\d{2}:\d{2}(?:\.\d+)?\s+)/
// Level keyword, optionally wrapped in dashes.
const LEVEL_RE =
  /^(?:-\s*)?(ERROR|WARNING|WARN|INFO|DEBUG|TRACE|CRITICAL|FATAL)\b(?:\s*-(?!-)\s*)?/i
// First [tag] anywhere in the remainder.
const TAG_RE = /\[([^\]]+)\]/
// Time-of-day extraction for display + bucketing.
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
  if (tagMatch && tagMatch.index != null) {
    subsystem = tagMatch[1].trim()
    // Pull the first tag out of the message body; it lives in its own column.
    message = (remaining.slice(0, tagMatch.index) + remaining.slice(tagMatch.index + tagMatch[0].length)).trim()
  }

  return { ts, level, subsystem, message, raw: line }
}

// Time-of-day in milliseconds, used only for relative histogram bucketing.
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

// Distribute records across `buckets` columns by timestamp when the records span
// a positive time range; otherwise fall back to even index bucketing (covers the
// empty-timestamp and midnight-wrap cases without crashing).
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

// Distinct subsystem tags with counts, ordered by count desc then name.
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
