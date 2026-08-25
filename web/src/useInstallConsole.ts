import { useCallback, useMemo, useRef, useState } from 'react'
import { getLogs } from './api'
import { usePolling } from './usePolling'
import { parseProgressBars, type TqdmBar } from './progressBarParsing'
import type { InstallationJob } from './types'

export interface ConsoleLine {
  id: number
  state: 'done' | 'active' | 'pending' | 'error'
  time: string
  op: string
  msg: string
}

const MAX_RAW_LINES = 400
const INSTALL_TAG = '[install]'

// Rules are ordered. Keep op names synchronized with `stages.ts`.
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

/**
 * @fn deriveOp(msg: string): string
 * @brief Map log text to the first matching display stage.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param msg Install log text after the tag.
 * @return An operation from OP_RULES, or INSTALL when no rule matches.
 */
function deriveOp(msg: string): string {
  for (const [re, op] of OP_RULES) {
    if (re.test(msg)) return op
  }
  return 'INSTALL'
}

/**
 * @fn deriveMessage(afterTag: string): string
 * @brief Remove a leading log severity from install display text.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param afterTag Text following the install tag.
 * @return Trimmed message text.
 */
function deriveMessage(afterTag: string): string {
  return afterTag.replace(/^\s*-?\s*(INFO|DEBUG|TRACE|WARN(?:ING)?|ERROR|CRITICAL|FATAL)\b\s*[:-]?\s*/i, '').trim()
}

/**
 * @fn mapInstallLines(rawLines: string[], processing: boolean): ConsoleLine[]
 * @brief Filter install log messages and mark the latest record for display.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param rawLines Bounded raw log window.
 * @param processing Mark the final row active when that row is not an error.
 * @return Display rows with IDs relative to the input window, not persistent log IDs.
 */
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

/**
 * @fn activeOpOf(lines: ConsoleLine[]): string | null
 * @brief Find the latest active or failed display stage.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param lines Console rows in time order.
 * @return The latest matching operation, or null.
 */
export function activeOpOf(lines: ConsoleLine[]): string | null {
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].state === 'active' || lines[i].state === 'error') return lines[i].op
  }
  return null
}

/**
 * @fn deriveActiveOp(rawLines: string[]): string | null
 * @brief Infer the displayed stage from install log text.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param rawLines Raw log window, which may also contain other subsystems.
 * @return The active or failed operation, or null when no install messages remain.
 */
export function deriveActiveOp(rawLines: string[]): string | null {
  return activeOpOf(mapInstallLines(rawLines, true))
}

/**
 * @fn useInstallConsole(activeJobId: string | null, active: boolean): {
 *   lines: ConsoleLine[]; rawLines: string[]
 * }
 * @brief Maintain a bounded log window for the installation console.
 * @author Alex (<https://github.com/lextpf>)
 *
 * A job change resets the byte offset; a server reset replaces the buffer.
 * The log endpoint is shared, so a browser job ID does not isolate messages by installation.
 *
 * @param activeJobId Browser job ID used to reset the window on the next poll.
 * @param active Enable one-second polling and mark the final row active unless it is an error.
 * @return Parsed install rows and the shared raw log window.
 */
export function useInstallConsole(activeJobId: string | null, active: boolean): {
  lines: ConsoleLine[]
  rawLines: string[]
} {
  const [rawLines, setRawLines] = useState<string[]>([])
  const offsetRef = useRef<number | undefined>(undefined)
  const lastJobRef = useRef<string | null>(null)

  /**
   * @fn poll(): Promise<void>
   * @brief Merge incremental log responses and retain at most 400 raw lines.
   * @author Alex (<https://github.com/lextpf>)
   */
  const poll = useCallback(async () => {
    // Reset the window when the active job changes.
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
  /**
   * @brief Current operation progress.
   *
   * Values are in [0, 100], or null when indeterminate.
   */
  pct: number | null
  label: string
  tone: 'normal' | 'done' | 'error'
  indeterminate: boolean
}

/**
 * @fn clampPct(n: number): number
 * @brief Round display progress and limit it to the percentage scale.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param n Finite percentage before rounding.
 * @return An integer from zero through 100.
 */
function clampPct(n: number): number {
  return Math.max(0, Math.min(100, Math.round(n)))
}

/**
 * @fn computeInstallProgress(job: InstallationJob, rawLines: string[]): InstallProgress
 * @brief Select progress from the source available for the current job state.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Uploads use XHR progress. Processing uses the first parsed log progress bar.
 * A missing bar or unknown total leaves progress indeterminate.
 *
 * @param job Browser installation state.
 * @param rawLines Current log window used while processing.
 * @return Display progress, label, and terminal tone.
 */
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

  // The list is empty before the first tqdm line arrives, so there is often no bar.
  const bars = parseProgressBars(rawLines, 'salma')
  const bar: TqdmBar | undefined = bars.length > 0 ? bars[0] : undefined
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
