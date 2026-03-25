import type { AppConfig, Mo2Status, FomodEntry, FomodDetail, LogsResponse, TestStatus, FomodScanStatus, PluginActionResult, PluginActionStatus, InstallStatus } from './types'

const BASE = ''

const DEFAULT_TIMEOUT_MS = 30_000

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    const timeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
    const signal = init?.signal
      ? AbortSignal.any([init.signal, timeoutSignal])
      : timeoutSignal
    res = await fetch(`${BASE}${url}`, { ...init, signal })
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

export async function getFomod(name: string): Promise<FomodDetail> {
  return fetchJson(`/api/mo2/fomods/${encodeURIComponent(name)}`)
}

async function fetchVoid(url: string, init?: RequestInit): Promise<void> {
  let res: Response
  try {
    const timeoutSignal = AbortSignal.timeout(DEFAULT_TIMEOUT_MS)
    const signal = init?.signal
      ? AbortSignal.any([init.signal, timeoutSignal])
      : timeoutSignal
    res = await fetch(`${BASE}${url}`, { ...init, signal })
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

export async function getInstallStatus(): Promise<InstallStatus> {
  return fetchJson('/api/installation/status/current')
}
