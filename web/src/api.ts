// The only place the dashboard talks to mo2-server. Every page and hook calls
// through the exported functions below. Do not add an ad-hoc fetch elsewhere:
// the CSRF header, the single retry and the timeout live here and nowhere else.
//
// One deliberate exception. The upload in useInstallation.ts uses a raw
// XMLHttpRequest to get upload-progress events, and re-implements only the CSRF
// header, so it has neither the retry nor the timeout.
//
// Request path:
//
//   caller          api.ts              /api/csrf-token       /api/<route>
//     |               |                        |                   |
//     |-- getConfig ->|                        |                   |
//     |               |-- cached token? -------|                   |
//     |               |<-- {token} ------------|  (fetched once per page load)
//     |               |------ request + X-Salma-Csrf ------------->|
//     |               |<----- 403 {"error":"csrf token missing or invalid"}
//     |               |-- drop cached token, fetch a new one ----->|
//     |               |------ same request, one more time -------->|
//     |               |<----- 200 {json} --------------------------|
//     |<-- value -----|
//
// CSRF contract:
//   - The token is fetched once and cached as a promise for the life of the
//     page. A failed fetch clears the cache, so the next call tries again.
//   - It is sent on every request, reads included.
//   - A 403 is retried only when the response body's `error` field equals
//     CSRF_INVALID_ERROR exactly. Any other 403 goes to the caller.
//   - The retry happens at most once. There is no other automatic retry.
//
// Timeout contract:
//   - Each attempt gets a fresh DEFAULT_TIMEOUT_MS budget. A caller-supplied
//     `init.signal` is combined with it through AbortSignal.any, so whichever
//     of the two fires first aborts the attempt.
//   - The caller's signal object is reused across the retry while the default
//     budget restarts, so a request that retries can run for up to
//     2 * DEFAULT_TIMEOUT_MS.
//   - The token fetch runs before the timeout signal is created, so a slow
//     /api/csrf-token is outside the budget.
//
// Failure contract. Every failure is a plain Error; nothing here rejects with a
// Response or a DOMException:
//   - timeout (either signal)       -> Error('Backend unavailable')
//   - network-level fetch TypeError -> Error('Backend unavailable')
//   - any other non-2xx status      -> Error(body.error) or Error(statusText)
//   - unparsable 2xx JSON body      -> Error('Invalid JSON in response from <url>')
// isFetchUnavailableError() separates "the backend is not answering" from "the
// backend answered and refused". A slow backend and a stopped one are
// indistinguishable: both surface as 'Backend unavailable'.
import type { AppConfig, Mo2Status, FomodEntry, FomodDetail, LogsResponse, TestStatus, FomodScanStatus, PluginActionResult, PluginActionStatus, InstallStatus } from './types'

/** Prefix for every request path. Empty, so Vite's dev proxy handles /api. */
const BASE = ''

/** Per-attempt request budget in milliseconds. See the timeout contract above. */
const DEFAULT_TIMEOUT_MS = 30_000

const CSRF_HEADER = 'X-Salma-Csrf'
/**
 * The exact `error` string SecurityMiddleware returns with a 403 when the token
 * is missing or stale. Matched literally: a 403 with any other body is a real
 * refusal and is not retried. Keep in step with src/SecurityMiddleware.cpp.
 */
const CSRF_INVALID_ERROR = 'csrf token missing or invalid'

// Cached as the promise, not the resolved string, so concurrent first calls
// share one /api/csrf-token request instead of racing several.
let csrfTokenPromise: Promise<string> | null = null

async function fetchCsrfToken(): Promise<string> {
  const res = await fetch('/api/csrf-token')
  if (!res.ok) {
    throw new Error(`Failed to fetch CSRF token (${res.status})`)
  }
  const body = await res.json()
  if (typeof body.token !== 'string' || body.token.length === 0) {
    throw new Error('CSRF token endpoint returned invalid body')
  }
  return body.token
}

/**
 * The cached CSRF token, fetching it on first use.
 *
 * Exported because useInstallation.ts needs the raw token for its XHR upload.
 * Rejects if /api/csrf-token is unreachable or returns a body without a
 * non-empty string `token`; the cache is cleared on rejection so a later call
 * retries rather than replaying the failure.
 */
export async function getCsrfToken(): Promise<string> {
  if (!csrfTokenPromise) {
    csrfTokenPromise = fetchCsrfToken().catch((err) => {
      csrfTokenPromise = null
      throw err
    })
  }
  return csrfTokenPromise
}

function clearCsrfToken(): void {
  csrfTokenPromise = null
}

/**
 * One attempt: attach the CSRF header, arm the timeout, send.
 *
 * The token is awaited before the timeout signal is built, so time spent
 * fetching the token does not count against DEFAULT_TIMEOUT_MS.
 */
async function performRequest(url: string, init?: RequestInit): Promise<Response> {
  const headers = new Headers(init?.headers as HeadersInit | undefined)
  headers.set(CSRF_HEADER, await getCsrfToken())
  const timeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
  const signal = init?.signal
    ? AbortSignal.any([init.signal, timeoutSignal])
    : timeoutSignal
  return fetch(`${BASE}${url}`, { ...init, headers, signal })
}

/**
 * performRequest plus the single CSRF retry.
 *
 * The 403 body is read from a clone, so the caller still gets an unread body if
 * the response is handed back. `init` is reused verbatim on the retry, which is
 * safe only because every body sent from this module is a JSON string, never a
 * one-shot stream. Reuse also means a caller-supplied AbortSignal keeps counting
 * down across both attempts, and an already-expired one aborts the retry at
 * once.
 *
 * When a retry happened, returns the second attempt's response even if that one
 * is also a 403. Never retries more than once.
 */
async function performRequestWithCsrfRetry(url: string, init?: RequestInit): Promise<Response> {
  let res = await performRequest(url, init)
  if (res.status === 403) {
    const peek = await res.clone().json().catch(() => null)
    if (peek?.error === CSRF_INVALID_ERROR) {
      clearCsrfToken()
      res = await performRequest(url, init)
    }
  }
  return res
}

/**
 * Send a request and decode a JSON body of type T.
 *
 * T is asserted, not validated: the body is whatever the server sent. Applies
 * the failure contract at the top of this file. Never returns undefined; it
 * either resolves with the decoded body or throws an Error.
 */
async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await performRequestWithCsrfRetry(url, init)
  } catch (error) {
    if (error instanceof DOMException && error.name === 'TimeoutError') {
      throw new Error('Backend unavailable')
    }
    // fetch() throws TypeError for network failures (DNS, connection refused, CORS).
    // Re-wrap as a recognizable error so callers can distinguish backend-down from bugs.
    if (error instanceof TypeError && isFetchNetworkError(error)) {
      throw new Error('Backend unavailable')
    }
    throw error
  }
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }))
    const message = body.error || res.statusText
    throw new Error(message)
  }
  try {
    return await res.json()
  } catch {
    throw new Error(`Invalid JSON in response from ${url}`)
  }
}

/** Detect network-level TypeErrors thrown by fetch (not coding bugs). */
function isFetchNetworkError(error: TypeError): boolean {
  const msg = error.message.toLowerCase()
  return msg.includes('failed to fetch') || msg.includes('network') || msg.includes('load failed')
}

/**
 * True when the failure means "the backend did not answer", not "the backend
 * refused". Matches a raw network TypeError and the Error('Backend unavailable')
 * this module rewraps a network failure or a timeout as.
 *
 * A slow backend that hits the timeout looks the same as a stopped one, so a
 * caller that retries on this must bound its attempts.
 */
export function isFetchUnavailableError(error: unknown): boolean {
  if (error instanceof TypeError && isFetchNetworkError(error)) return true
  if (error instanceof Error && error.message === 'Backend unavailable') return true
  return false
}

export async function getConfig(): Promise<AppConfig> {
  return fetchJson('/api/config')
}

export async function putConfig(config: Partial<Pick<AppConfig, 'mo2ModsPath'>>): Promise<AppConfig> {
  return fetchJson('/api/config', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  })
}

export async function getMo2Status(): Promise<Mo2Status> {
  return fetchJson('/api/mo2/status')
}

export async function scanFomods(): Promise<FomodScanStatus> {
  return fetchJson('/api/mo2/fomods/scan', { method: 'POST' })
}

export async function getFomodScanStatus(): Promise<FomodScanStatus> {
  return fetchJson('/api/mo2/fomods/scan/status')
}

export async function deployPlugin(): Promise<PluginActionResult> {
  return fetchJson('/api/plugin/deploy', { method: 'POST' })
}

export async function purgePlugin(): Promise<PluginActionResult> {
  return fetchJson('/api/plugin/purge', { method: 'POST' })
}

export async function listFomods(): Promise<FomodEntry[]> {
  return fetchJson('/api/mo2/fomods')
}

/**
 * One cached FOMOD record, by its file stem.
 *
 * Runs on an 8 second budget rather than the module default, because the Library
 * blocks two columns on this one call and a slow record is better reported than
 * waited on. The tighter signal wins over the 30 second default, and a breach
 * surfaces as Error('Backend unavailable'), the same message a dead server
 * produces, so the caller cannot tell them apart.
 *
 * useRecordDetail.ts retries this up to MAX_RETRIES times, 2 seconds apart, so
 * the worst-case wait is that multiple of the 8 seconds. Changing this budget
 * changes that total.
 *
 * `name` is URL-encoded here; pass the raw stem, not an encoded one.
 */
export async function getFomod(name: string): Promise<FomodDetail> {
  return fetchJson(`/api/mo2/fomods/${encodeURIComponent(name)}`, {
    signal: AbortSignal.timeout(8000),
  })
}

/**
 * fetchJson for a route whose success body carries nothing worth reading.
 *
 * Same failure contract as fetchJson except the last case: a 2xx body is never
 * parsed, so a malformed success body cannot throw here.
 */
async function fetchVoid(url: string, init?: RequestInit): Promise<void> {
  let res: Response
  try {
    res = await performRequestWithCsrfRetry(url, init)
  } catch (error) {
    if (error instanceof DOMException && error.name === 'TimeoutError') {
      throw new Error('Backend unavailable')
    }
    if (error instanceof TypeError && isFetchNetworkError(error)) {
      throw new Error('Backend unavailable')
    }
    throw error
  }
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }))
    const message = body.error || res.statusText
    throw new Error(message)
  }
}

export async function deleteFomod(name: string): Promise<void> {
  await fetchVoid(`/api/mo2/fomods/${encodeURIComponent(name)}`, { method: 'DELETE' })
}

/**
 * Read the salma log.
 *
 * Two modes, chosen by `offset`:
 *   - omitted: full mode. Returns the last `lines` lines of the file and the
 *     counts for those lines only.
 *   - a byte offset: incremental mode. Returns the complete lines written after
 *     that byte, capped to the last `lines` of them, plus counts for just those.
 *     Incremental counts are deltas the caller adds to a running total.
 *
 * `nextOffset` is the byte to pass next. `reset: true` means the file shrank
 * (it was cleared or rotated); the caller must drop its buffer and its offset
 * and re-read in full mode. `lines` are raw log lines with no trailing newline.
 */
export async function getLogs(lines = 100, offset?: number): Promise<LogsResponse> {
  const params = offset != null ? `lines=${lines}&offset=${offset}` : `lines=${lines}`
  return fetchJson(`/api/logs?${params}`)
}

/** getLogs against the test-harness log. Same two modes and same reset rule. */
export async function getTestLogs(lines = 100, offset?: number): Promise<LogsResponse> {
  const params = offset != null ? `lines=${lines}&offset=${offset}` : `lines=${lines}`
  return fetchJson(`/api/logs/test?${params}`)
}

export async function clearLogs(source: 'salma' | 'test'): Promise<{ success: boolean }> {
  const path = source === 'test' ? '/api/logs/clear/test' : '/api/logs/clear'
  return fetchJson(path, { method: 'POST' })
}

/**
 * Start the Python test harness on the server.
 *
 * `args` is appended to the harness command line, so it reaches a spawned
 * process. The server whitelists it to letters, digits, space, underscore,
 * hyphen and dot, and rejects any ".." sequence; anything else comes back as a
 * 400 whose message this module throws as an Error. Do not pre-escape it here.
 *
 * Returns immediately with the starting status. Poll getTestStatus for the run.
 */
export async function runTests(args?: string): Promise<TestStatus> {
  return fetchJson('/api/test/run', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ args: args || '' }),
  })
}

export async function getTestStatus(): Promise<TestStatus> {
  return fetchJson('/api/test/status')
}

export async function getPluginActionStatus(): Promise<PluginActionStatus> {
  return fetchJson('/api/plugin/status')
}

/**
 * The status of the install the server is running right now.
 *
 * The server holds one background job, not a map of jobs, and ignores the path
 * segment, so "current" is not a job id and the answer is not scoped to the
 * caller's upload. Two browser tabs installing at once read the same record.
 * Do not build per-job polling on this route.
 */
export async function getInstallStatus(): Promise<InstallStatus> {
  return fetchJson('/api/installation/status/current')
}
