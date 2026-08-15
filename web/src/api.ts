/**
 * @brief Centralize dashboard requests and CSRF handling.
 * @author Alex (<https://github.com/lextpf>)
 *
 * ### :material-refresh: Token retries
 *
 * API requests include the cached CSRF token. A matching 403 clears the token and retries once.
 * Token retrieval uses its own unauthenticated request.
 *
 * ### :material-timer-outline: Request limits
 *
 * Each attempt has a 30-second budget. Token retrieval is outside this budget.
 * Network failures and timeouts become `Error('Backend unavailable')`.
 *
 * The upload path in `useInstallation.ts` uses XHR for progress events. It has no retry or timeout.
 */
import type { AppConfig, Mo2Status, FomodEntry, FomodDetail, LogsResponse, TestStatus, FomodScanStatus, PluginActionResult, PluginActionStatus, InstallStatus } from './types'

const BASE = ''

const DEFAULT_TIMEOUT_MS = 30_000

const CSRF_HEADER = 'X-Salma-Csrf'
// Keep this exact value synchronized with `SecurityMiddleware.cpp`.
const CSRF_INVALID_ERROR = 'csrf token missing or invalid'

// Cache the promise so concurrent callers share one token request.
let csrfTokenPromise: Promise<string> | null = null

/**
 * @fn fetchCsrfToken(): Promise<string>
 * @brief Load and validate the token before caching it.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return A non-empty token; invalid bodies and unsuccessful responses reject.
 */
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
 * @fn getCsrfToken(): Promise<string>
 * @brief Share one CSRF token request across concurrent callers.
 * @author Alex (<https://github.com/lextpf>)
 *
 * A failed request clears the cache. Later callers can retry token retrieval.
 * @return The non-empty token used by API requests and XHR uploads.
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

/**
 * @fn clearCsrfToken(): void
 * @brief Invalidate the shared token before a rejected request is retried.
 * @author Alex (<https://github.com/lextpf>)
 */
function clearCsrfToken(): void {
  csrfTokenPromise = null
}

/**
 * @fn performRequest(url: string, init?: RequestInit): Promise<Response>
 * @brief Attach the shared token and a timeout to one request.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The 30-second timeout starts after token retrieval. The caller signal is also retained.
 *
 * @param url Same-origin API path.
 * @param init Optional request settings and caller abort signal.
 * @return The response, including unsuccessful HTTP statuses.
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
 * @fn performRequestWithCsrfRetry(url: string, init?: RequestInit): Promise<Response>
 * @brief Refresh the token once when the server reports a CSRF mismatch.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Only the exact CSRF error triggers a retry. Reading a clone preserves the response body.
 *
 * @param url Same-origin API path.
 * @param init Optional request settings; the body must be reusable.
 * @return The original response or the single retry response.
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
 * @fn fetchJson<T>(url: string, init?: RequestInit): Promise<T>
 * @brief Decode API responses and normalize request failures.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Timeouts and recognized network failures reject with "Backend unavailable".
 * Other HTTP errors use the server message. Invalid successful JSON rejects separately.
 *
 * @tparam T Expected response type.
 * @param url Same-origin API path.
 * @param init Optional request settings; the body must be reusable.
 * @return Parsed JSON asserted as T without runtime schema validation.
 */
async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await performRequestWithCsrfRetry(url, init)
  } catch (error) {
    if (error instanceof DOMException && error.name === 'TimeoutError') {
      throw new Error('Backend unavailable')
    }
    // Normalize fetch network failures for availability checks.
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

/**
 * @fn isFetchNetworkError(error: TypeError): boolean
 * @brief Recognize browser fetch failures by their message.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param error TypeError raised during a request.
 * @return True for the supported network-error messages.
 */
function isFetchNetworkError(error: TypeError): boolean {
  const msg = error.message.toLowerCase()
  return msg.includes('failed to fetch') || msg.includes('network') || msg.includes('load failed')
}

/**
 * @fn isFetchUnavailableError(error: unknown): boolean
 * @brief Classify failures that availability retries can handle.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Timeouts and stopped servers are indistinguishable. Callers must bound retries.
 * @param error The rejected request value.
 * @return True for normalized availability failures and fetch network errors.
 */
export function isFetchUnavailableError(error: unknown): boolean {
  if (error instanceof TypeError && isFetchNetworkError(error)) return true
  if (error instanceof Error && error.message === 'Backend unavailable') return true
  return false
}

/**
 * @fn getConfig(): Promise<AppConfig>
 * @brief Read the configured mod path and its validation state.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The current server configuration.
 */
export async function getConfig(): Promise<AppConfig> {
  return fetchJson('/api/config')
}

/**
 * @fn putConfig(config: Partial<Pick<AppConfig, 'mo2ModsPath'>>): Promise<AppConfig>
 * @brief Submit a mod-path update for server validation.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param config Fields to update; an omitted mod path is retained.
 * @return The resulting server configuration.
 */
export async function putConfig(config: Partial<Pick<AppConfig, 'mo2ModsPath'>>): Promise<AppConfig> {
  return fetchJson('/api/config', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  })
}

/**
 * @fn getMo2Status(): Promise<Mo2Status>
 * @brief Read plugin availability and mod-directory status.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The server view of the configured MO2 installation.
 */
export async function getMo2Status(): Promise<Mo2Status> {
  return fetchJson('/api/mo2/status')
}

/**
 * @fn scanFomods(): Promise<FomodScanStatus>
 * @brief Start a background scan of installed mods.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The initial scan status; use getFomodScanStatus to observe completion.
 */
export async function scanFomods(): Promise<FomodScanStatus> {
  return fetchJson('/api/mo2/fomods/scan', { method: 'POST' })
}

/**
 * @fn getFomodScanStatus(): Promise<FomodScanStatus>
 * @brief Read the shared scan job without starting another scan.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The current or most recent scan status.
 */
export async function getFomodScanStatus(): Promise<FomodScanStatus> {
  return fetchJson('/api/mo2/fomods/scan/status')
}

/**
 * @fn deployPlugin(): Promise<PluginActionResult>
 * @brief Start deployment to the configured plugin directory.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The accepted action; poll getPluginActionStatus for completion.
 */
export async function deployPlugin(): Promise<PluginActionResult> {
  return fetchJson('/api/plugin/deploy', { method: 'POST' })
}

/**
 * @fn purgePlugin(): Promise<PluginActionResult>
 * @brief Start removal of the deployed plugin.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The accepted action; poll getPluginActionStatus for completion.
 */
export async function purgePlugin(): Promise<PluginActionResult> {
  return fetchJson('/api/plugin/purge', { method: 'POST' })
}

/**
 * @fn listFomods(): Promise<FomodEntry[]>
 * @brief List cached record summaries for the library.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return Record metadata; full details require getFomod.
 */
export async function listFomods(): Promise<FomodEntry[]> {
  return fetchJson('/api/mo2/fomods')
}

/**
 * @fn getFomod(name: string): Promise<FomodDetail>
 * @brief Apply the detail route timeout to one cache record.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The 8-second signal starts before token retrieval and is reused on a CSRF retry.
 * It aborts the detail request, but it cannot interrupt token retrieval.
 * @param name The record file stem.
 * @return The cached record.
 */
export async function getFomod(name: string): Promise<FomodDetail> {
  return fetchJson(`/api/mo2/fomods/${encodeURIComponent(name)}`, {
    signal: AbortSignal.timeout(8000),
  })
}

/**
 * @fn fetchVoid(url: string, init?: RequestInit): Promise<void>
 * @brief Accept successful responses without reading their body.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Failure handling matches fetchJson. An empty successful response is valid.
 *
 * @param url Same-origin API path.
 * @param init Optional request settings; the body must be reusable.
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

/**
 * @fn deleteFomod(name: string): Promise<void>
 * @brief Delete one cached selection record.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param name Unencoded record file stem.
 */
export async function deleteFomod(name: string): Promise<void> {
  await fetchVoid(`/api/mo2/fomods/${encodeURIComponent(name)}`, { method: 'DELETE' })
}

/**
 * @fn getLogs(lines = 100, offset?: number): Promise<LogsResponse>
 * @brief Support full and byte-offset log reads with one response contract.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Omit @p offset for a tail read. Incremental counts are deltas. If `reset` is true, discard the
 * prior buffer and offset. `nextOffset` is a byte offset.
 * @param lines Maximum returned line count.
 * @param offset Optional byte offset for an incremental read.
 * @return Raw lines, counts, and the next offset.
 */
export async function getLogs(lines = 100, offset?: number): Promise<LogsResponse> {
  const params = offset != null ? `lines=${lines}&offset=${offset}` : `lines=${lines}`
  return fetchJson(`/api/logs?${params}`)
}

/**
 * @fn getTestLogs(lines = 100, offset?: number): Promise<LogsResponse>
 * @brief Read harness logs with the same offset contract as server logs.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param lines Maximum returned line count.
 * @param offset Optional byte offset; omit for a tail read.
 * @return Lines, counts, and reset metadata as described by getLogs.
 */
export async function getTestLogs(lines = 100, offset?: number): Promise<LogsResponse> {
  const params = offset != null ? `lines=${lines}&offset=${offset}` : `lines=${lines}`
  return fetchJson(`/api/logs/test?${params}`)
}

/**
 * @fn clearLogs(source: 'salma' | 'test'): Promise<{ success: boolean }>
 * @brief Truncate the selected server log.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param source Server or harness log to clear.
 * @return The server acknowledgement.
 */
export async function clearLogs(source: 'salma' | 'test'): Promise<{ success: boolean }> {
  const path = source === 'test' ? '/api/logs/clear/test' : '/api/logs/clear'
  return fetchJson(path, { method: 'POST' })
}

/**
 * @fn runTests(args?: string): Promise<TestStatus>
 * @brief Submit raw harness arguments for server-side validation.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The server validates @p args before process creation. Do not pre-escape the value.
 * @param args Optional harness arguments.
 * @return The initial status. Poll `getTestStatus` for updates.
 */
export async function runTests(args?: string): Promise<TestStatus> {
  return fetchJson('/api/test/run', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ args: args || '' }),
  })
}

/**
 * @fn getTestStatus(): Promise<TestStatus>
 * @brief Read the shared harness process status.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The current process state and any available exit information.
 */
export async function getTestStatus(): Promise<TestStatus> {
  return fetchJson('/api/test/status')
}

/**
 * @fn getPluginActionStatus(): Promise<PluginActionStatus>
 * @brief Read deployment or purge progress.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return The current or most recent plugin action status.
 */
export async function getPluginActionStatus(): Promise<PluginActionStatus> {
  return fetchJson('/api/plugin/status')
}

/**
 * @fn getInstallStatus(): Promise<InstallStatus>
 * @brief Read the server-wide installation job.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The route reports one global job. It is not scoped to a browser or upload.
 * @return The shared install status.
 */
export async function getInstallStatus(): Promise<InstallStatus> {
  return fetchJson('/api/installation/status/current')
}
