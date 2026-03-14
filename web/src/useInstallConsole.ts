import { useCallback, useMemo, useRef, useState } from 'react'
import { getLogs } from './api'
import { usePolling } from './usePolling'
import { parseProgressBars } from './progressBarParsing'
import type { InstallationJob } from './types'

// One rendered op-line in the install console stream.
export interface ConsoleLine {
  id: number
  state: 'done' | 'active' | 'pending' | 'error'
  time: string
  op: string
  msg: string
}

const MAX_RAW_LINES = 400
const INSTALL_TAG = '[install]'

const OP_RULES: [RegExp, string][] = [
  [/extract/i, 'EXTRACT'],
  [/parse|moduleconfig/i, 'PARSE'],
  [/cache|tier-1|meta\.ini/i, 'CACHE'],
  [/propagat/i, 'PROPAGATE'],
  [/\bcsp\b|solver/i, 'CSP'],
  [/simulat/i, 'SIMULATE'],
  [/infer|select/i, 'INFER'],
  [/copy|file operation/i, 'COPY'],
  [/writ|output|\.json/i, 'WRITE'],
  [/deploy/i, 'DEPLOY'],
  [/clean/i, 'CLEANUP'],
  [/detect|structure|content root/i, 'DETECT'],
  [/complet|success|installed/i, 'DONE'],
  [/fail|error|abort|cancel/i, 'ERROR'],
]

const ERROR_RE = /\b(ERROR|FAIL|FAILED|CRITICAL|FATAL|ABORT|CANCEL)/i
const TIME_RE = /(\d{2}:\d{2}:\d{2})/

function deriveOp(msg: string): string {
  for (const [re, op] of OP_RULES) {
    if (re.test(msg)) return op
  }
  return 'INSTALL'
}

// Pull the human message that follows the [install] tag, dropping a leading
// log level if one is present so the op column does not repeat it.
function deriveMessage(afterTag: string): string {
  return afterTag.replace(/^\s*-?\s*(INFO|DEBUG|TRACE|WARN(?:ING)?|ERROR|CRITICAL|FATAL)\b\s*[:-]?\s*/i, '').trim()
}

// Map the raw salma log tail into install op-lines. Only [install]-tagged lines
// are kept; the freshest line is marked active (unless it is an error) so the
// stream reads as live while a job is processing.
function mapInstallLines(rawLines: string[], processing: boolean): ConsoleLine[] {
  const out: ConsoleLine[] = []
  for (let i = 0; i < rawLines.length; i++) {
    const line = rawLines[i]
    const tagAt = line.indexOf(INSTALL_TAG)
    if (tagAt < 0) continue
    const afterTag = line.slice(tagAt + INSTALL_TAG.length)
    const msg = deriveMessage(afterTag)
    if (!msg) continue
    const isError = ERROR_RE.test(afterTag)
    out.push({
      id: i,
      state: isError ? 'error' : 'done',
      time: (line.match(TIME_RE)?.[1]) ?? '-',
      op: deriveOp(afterTag),
      msg,
    })
  }
  if (processing && out.length > 0) {
    const last = out[out.length - 1]
    if (last.state !== 'error') last.state = 'active'
  }
  return out
}

// Polls the salma log tail (~1s) and projects the [install] stream for the
// active job. Returns the mapped op-lines plus the raw window so the footer can
// reuse parseProgressBars over the same data.
export function useInstallConsole(activeJobId: string | null, active: boolean): {
  lines: ConsoleLine[]
  rawLines: string[]
} {
  const [rawLines, setRawLines] = useState<string[]>([])
  const offsetRef = useRef<number | undefined>(undefined)
  const lastJobRef = useRef<string | null>(null)

  const poll = useCallback(async () => {
    // Reset the follow window whenever the active job changes.
    if (lastJobRef.current !== activeJobId) {
      lastJobRef.current = activeJobId
      offsetRef.current = undefined
      setRawLines([])
    }
    try {
      const res = await getLogs(MAX_RAW_LINES, offsetRef.current)
      if (res.reset || offsetRef.current === undefined) {
        setRawLines(res.lines)
      } else if (res.lines.length > 0) {
        setRawLines(prev => {
          const merged = prev.concat(res.lines)
          return merged.length > MAX_RAW_LINES ? merged.slice(merged.length - MAX_RAW_LINES) : merged
        })
      }
      if (res.nextOffset !== undefined) offsetRef.current = res.nextOffset
    } catch (e) {
      console.warn('[install] failed to fetch console logs', e)
    }
  }, [activeJobId])

  usePolling(poll, 1000, active)

  const lines = useMemo(() => mapInstallLines(rawLines, active), [rawLines, active])

  return { lines, rawLines }
}

export interface InstallProgress {
  // 0..100, or null when indeterminate / not meaningful.
  pct: number | null
  label: string
  tone: 'normal' | 'done' | 'error'
  indeterminate: boolean
}

function clampPct(n: number): number {
  return Math.max(0, Math.min(100, Math.round(n)))
}

// Derive the footer fill/label/percent for a job. Upload uses the XHR progress;
// processing parses any [solver]/[scan] tqdm bar out of the log tail, otherwise
// falls back to an indeterminate stripe.
export function computeInstallProgress(job: InstallationJob, rawLines: string[]): InstallProgress {
  switch (job.status) {
    case 'completed':
      return { pct: 100, label: 'done', tone: 'done', indeterminate: false }
    case 'error':
      return { pct: null, label: job.error === 'Cancelled' ? 'cancelled' : 'error', tone: 'error', indeterminate: false }
    case 'pending':
      return { pct: null, label: 'queued', tone: 'normal', indeterminate: true }
    case 'uploading':
      return { pct: clampPct(job.uploadProgress ?? 0), label: 'uploading', tone: 'normal', indeterminate: false }
    default:
      break
  }

  // Processing: look for a parseable progress bar in the log tail.
  const bar = parseProgressBars(rawLines, 'salma')[0]
  if (bar) {
    let pct: number | null = null
    if (bar.current != null && bar.total != null && bar.total > 0) {
      pct = clampPct((bar.current / bar.total) * 100)
    } else if (bar.rawBar) {
      const m = bar.rawBar.match(/(\d+)%/)
      if (m) pct = clampPct(parseInt(m[1], 10))
    }
    const label = bar.tag === 'solver'
      ? 'inferring - solver'
      : bar.tag === 'scan'
        ? (bar.detail ? `scanning - ${bar.detail}` : 'scanning')
        : 'processing'
    if (pct != null) return { pct, label, tone: 'normal', indeterminate: false }
    return { pct: null, label, tone: 'normal', indeterminate: true }
  }

  const label = job.processingStatus ? job.processingStatus.replace(/\.+$/, '').toLowerCase() : 'installing'
  return { pct: null, label, tone: 'normal', indeterminate: true }
}
