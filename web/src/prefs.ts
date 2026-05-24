/**
 * Browser-local preferences, held in localStorage.
 *
 * All three live only in the browser. None is written to salma.json, which
 * persists exactly one key, the MO2 mods path. They do not follow the user to
 * another browser or machine, and clearing site data resets them to the
 * defaults below. Settings labels them as local for that reason.
 *
 * Their scope differs. Rail collapsed and tail logs only change how this
 * browser renders the dashboard. salma_test_args does more: useTestRunner reads
 * it back and POSTs it to /api/test/run, where it becomes the argument string
 * of a process the server spawns. Browser storage, server effect.
 *
 * Every getter returns a usable value when the key is absent, so no caller has
 * to handle a first run. localStorage access is synchronous and can throw in a
 * hardened browser profile; none of these functions guards against that today.
 */

const TAIL_KEY = 'salma_tail_logs'
const RAIL_KEY = 'salma_rail_collapsed'
/**
 * Exported because useTestRunner reads this key straight out of localStorage on
 * the run path instead of calling getTestArgs. Two readers, one spelling.
 */
export const TEST_ARGS_KEY = 'salma_test_args'

/** Rail folded to the 62px icon spine. Defaults to false (expanded). */
export function getRailCollapsed(): boolean {
  return localStorage.getItem(RAIL_KEY) === 'true'
}

export function setRailCollapsed(on: boolean): void {
  localStorage.setItem(RAIL_KEY, on ? 'true' : 'false')
}

/**
 * Auto-refresh the Logs stream. Defaults to on: only the literal 'false' turns
 * it off, so an absent key means on.
 */
export function getTailLogs(): boolean {
  return localStorage.getItem(TAIL_KEY) !== 'false'
}

export function setTailLogs(on: boolean): void {
  localStorage.setItem(TAIL_KEY, on ? 'true' : 'false')
}

/**
 * Extra arguments for the server-side test harness. Defaults to ''.
 *
 * Stored unvalidated. The server whitelists the string to letters, digits,
 * space, underscore, hyphen and dot before it spawns anything, so do not treat
 * validation here as the guard.
 */
export function getTestArgs(): string {
  return localStorage.getItem(TEST_ARGS_KEY) || ''
}

export function setTestArgs(v: string): void {
  localStorage.setItem(TEST_ARGS_KEY, v)
}
