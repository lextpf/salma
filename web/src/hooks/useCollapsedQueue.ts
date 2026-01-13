import { useState } from 'react'

const STORAGE_KEY = 'salma-queue-collapsed'

function readPref(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === 'collapsed'
  } catch {
    return false
  }
}

function writePref(collapsed: boolean): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, collapsed ? 'collapsed' : 'expanded')
  } catch {
    // localStorage unavailable (private mode, sandboxed iframe, etc.) - skip persist
  }
}

/**
 * Tri-input collapse logic for the install queue.
 *
 * Display priority:
 *  1. A user toggle issued in the current viewport era wins (so clicking
 *     expand on a short viewport still expands, until the viewport changes).
 *  2. If the caller signals the viewport is too short, force collapsed.
 *  3. Otherwise honor the persisted user preference (defaults to expanded).
 *
 * The session-level override is invalidated whenever the viewport transitions
 * between short and not-short - this way a window resize that crosses the
 * threshold always yields a fresh auto-decision rather than replaying a stale
 * click. The persisted preference itself is only updated by user toggles.
 */
export function useCollapsedQueue(
  forceCollapseDefault: boolean,
): [boolean, (next: boolean) => void] {
  const [pref, setPref] = useState<boolean>(readPref)
  const [override, setOverride] = useState<boolean | null>(null)
  const [prevForceShort, setPrevForceShort] = useState<boolean>(forceCollapseDefault)

  // Drop the session override when the viewport crosses the threshold.
  // Setting state during render is the React-recommended pattern for
  // derived state that needs to reset on a prop change, and it avoids
  // the setState-in-effect lint rule.
  if (prevForceShort !== forceCollapseDefault) {
    setOverride(null)
    setPrevForceShort(forceCollapseDefault)
  }

  const setCollapsed = (next: boolean) => {
    setPref(next)
    writePref(next)
    setOverride(next)
  }

  const collapsed = override !== null
    ? override
    : forceCollapseDefault || pref

  return [collapsed, setCollapsed]
}
