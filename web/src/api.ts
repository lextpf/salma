import type { AppConfig, Mo2Status, FomodEntry, LogsResponse, TestStatus, FomodScanStatus, PluginActionResult, PluginActionStatus, InstallStatus } from './types'

const BASE = ''

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await fetch(`${BASE}${url}`, init)
  } catch (error) {
    if (isFetchUnavailableError(error)) {
      throw new Error('Backend unavailable')
    }
    throw error
  }
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }))
    const message = body.error || res.statusText
    if (typeof message === 'string' && message.toLowerCase().includes('failed to fetch')) {
      throw new Error('Backend unavailable')
    }
    throw new Error(message)
  }
  return res.json()
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error ?? '')
}

export function isFetchUnavailableError(error: unknown): boolean {
  const message = errorMessage(error).toLowerCase()
  return (
    message.includes('failed to fetch') ||
    message.includes('networkerror') ||
    message.includes('network error') ||
    message.includes('load failed') ||
    message.includes('the network connection was lost')
  )
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

export async function getFomod(name: string): Promise<Record<string, unknown>> {
  return fetchJson(`/api/mo2/fomods/${encodeURIComponent(name)}`)
}

export async function deleteFomod(name: string): Promise<void> {
  await fetchJson(`/api/mo2/fomods/${encodeURIComponent(name)}`, { method: 'DELETE' })
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
