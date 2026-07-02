/**
 * @brief centralize dashboard requests and CSRF handling.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-refresh: token retries
 *
 * all requests include the cached CSRF token. a matching 403 clears the token and retries once.
 *
 * ### :material-timer-outline: request limits
 *
 * each attempt has a 30-second budget. token retrieval is outside this budget.
 * network failures and timeouts become `Error('Backend unavailable')`.
 *
 * the upload path in `useInstallation.ts` uses XHR for progress events. it has no retry or timeout.
 */
import type { AppConfig, Mo2Status, FomodEntry, FomodDetail, LogsResponse, TestStatus, FomodScanStatus, PluginActionResult, PluginActionStatus, InstallStatus } from './types'

const BASE = ''

const DEFAULT_TIMEOUT_MS = 30_000

const CSRF_HEADER = 'X-Salma-Csrf'
// keep this exact value synchronized with `SecurityMiddleware.cpp`.
const CSRF_INVALID_ERROR = 'csrf token missing or invalid'

// cache the promise so concurrent callers share one token request.
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
 * @fn getCsrfToken(): Promise<string>
 * @brief share one CSRF token request across concurrent callers.
 * @author Alex (https://github.com/lextpf)
 *
 * a failed request clears the cache. later callers can retry token retrieval.
 * @return the non-empty token used by API requests and XHR uploads.
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

// start the request budget after token retrieval.
async function performRequest(url: string, init?: RequestInit): Promise<Response> {
  const headers = new Headers(init?.headers as HeadersInit | undefined)
  headers.set(CSRF_HEADER, await getCsrfToken())
  const timeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
  const signal = init?.signal
    ? AbortSignal.any([init.signal, timeoutSignal])
    : timeoutSignal
  return fetch(`${BASE}${url}`, { ...init, headers, signal })
}

// clone the 403 body so the final response remains unread. request bodies must be reusable.
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

// assert the response type without runtime validation.
async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await performRequestWithCsrfRetry(url, init)
  } catch (error) {
    if (error instanceof DOMException && error.name === 'TimeoutError') {
      throw new Error('Backend unavailable')
    }
    // normalize fetch network failures for availability checks.
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

function isFetchNetworkError(error: TypeError): boolean {
  const msg = error.message.toLowerCase()
  return msg.includes('failed to fetch') || msg.includes('network') || msg.includes('load failed')
}

/**
 * @fn isFetchUnavailableError(error: unknown): boolean
 * @brief classify failures that availability retries can handle.
 * @author Alex (https://github.com/lextpf)
 *
 * timeouts and stopped servers are indistinguishable. callers must bound retries.
 * @param error the rejected request value.
 * @return true for normalized availability failures and fetch network errors.
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
 * @fn getFomod(name: string): Promise<FomodDetail>
 * @brief apply the detail route timeout to one cache record.
 * @author Alex (https://github.com/lextpf)
 *
 * this request has an 8-second budget. pass the unencoded file stem.
 * @param name the record file stem.
 * @return the cached record.
 */
export async function getFomod(name: string): Promise<FomodDetail> {
  return fetchJson(`/api/mo2/fomods/${encodeURIComponent(name)}`, {
    signal: AbortSignal.timeout(8000),
  })
}

// do not parse successful responses from routes without a response contract.
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
 * @fn getLogs(lines = 100, offset?: number): Promise<LogsResponse>
 * @brief support full and byte-offset log reads with one response contract.
 * @author Alex (https://github.com/lextpf)
 *
 * omit @p offset for a tail read. incremental counts are deltas. if `reset` is true, discard the
 * prior buffer and offset. `nextOffset` is a byte offset.
 * @param lines maximum returned line count.
 * @param offset optional byte offset for an incremental read.
 * @return raw lines, counts, and the next offset.
 */
export async function getLogs(lines = 100, offset?: number): Promise<LogsResponse> {
  const params = offset != null ? `lines=${lines}&offset=${offset}` : `lines=${lines}`
  return fetchJson(`/api/logs?${params}`)
}

export async function getTestLogs(lines = 100, offset?: number): Promise<LogsResponse> {
  const params = offset != null ? `lines=${lines}&offset=${offset}` : `lines=${lines}`
  return fetchJson(`/api/logs/test?${params}`)
}

export async function clearLogs(source: 'salma' | 'test'): Promise<{ success: boolean }> {
  const path = source === 'test' ? '/api/logs/clear/test' : '/api/logs/clear'
  return fetchJson(path, { method: 'POST' })
}

/**
 * @fn runTests(args?: string): Promise<TestStatus>
 * @brief submit raw harness arguments for server-side validation.
 * @author Alex (https://github.com/lextpf)
 *
 * the server validates @p args before process creation. do not pre-escape the value.
 * @param args optional harness arguments.
 * @return the initial status. poll `getTestStatus` for updates.
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
 * @fn getInstallStatus(): Promise<InstallStatus>
 * @brief expose the server-wide job instead of browser-local upload state.
 * @author Alex (https://github.com/lextpf)
 *
 * the route reports one global job. it is not scoped to a browser or upload.
 * @return the shared install status.
 */
export async function getInstallStatus(): Promise<InstallStatus> {
  return fetchJson('/api/installation/status/current')
}
