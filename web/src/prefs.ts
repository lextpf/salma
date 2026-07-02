/**
 * @brief store browser-local dashboard preferences.
 * @author Alex (https://github.com/lextpf)
 *
 * these values do not persist to `salma.json`. storage access can throw in restricted profiles.
 */

const TAIL_KEY = 'salma_tail_logs'
const RAIL_KEY = 'salma_rail_collapsed'
export const TEST_ARGS_KEY = 'salma_test_args'

export function getRailCollapsed(): boolean {
  return localStorage.getItem(RAIL_KEY) === 'true'
}

export function setRailCollapsed(on: boolean): void {
  localStorage.setItem(RAIL_KEY, on ? 'true' : 'false')
}

export function getTailLogs(): boolean {
  return localStorage.getItem(TAIL_KEY) !== 'false'
}

export function setTailLogs(on: boolean): void {
  localStorage.setItem(TAIL_KEY, on ? 'true' : 'false')
}

/**
 * @fn getTestArgs(): string
 * @brief retrieve untrusted browser-local harness arguments.
 * @author Alex (https://github.com/lextpf)
 *
 * server-side validation remains the security boundary.
 */
export function getTestArgs(): string {
  return localStorage.getItem(TEST_ARGS_KEY) || ''
}

export function setTestArgs(v: string): void {
  localStorage.setItem(TEST_ARGS_KEY, v)
}
