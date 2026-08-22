// Shared formatting helpers, used by the Library inspector and VFS tree and by
// the Install screen's job rows. They live outside the .tsx files so those stay
// single-export and fast-refresh friendly.

/**
 * Binary sizes, the way Mod Organizer 2 writes them: KiB / MiB / GiB. salma
 * sits beside MO2, so the units should match what the user reads one window
 * over.
 *
 * Three rules decide the decimal, in this order:
 *   1. A value of 100 or more is rounded to a whole number, because at three
 *      significant figures the decimal is noise.
 *   2. A value that rounds to a whole tenth is printed without the decimal.
 *   3. Everything else gets exactly one decimal.
 * So 41.3 MiB, but 48 MiB rather than 48.0 MiB, and 413 MiB with no decimal
 * at all. Under 1024 bytes the value is printed as bytes with no unit scaling.
 *
 * Returns 'n/a' for undefined, non-finite or negative input, so a caller can
 * pass a possibly-absent field straight through.
 */
export function formatSize(bytes?: number): string {
  if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes < 0) {
    return 'n/a'
  }
  if (bytes < 1024) return `${bytes} B`
  const units: [number, string][] = [
    [1024 ** 3, 'GiB'],
    [1024 ** 2, 'MiB'],
    [1024, 'KiB'],
  ]
  for (const [scale, unit] of units) {
    if (bytes >= scale) {
      const v = bytes / scale
      // Rules 1 and 2 above share one branch: both end in a whole number.
      const text = v >= 100 || Number.isInteger(Math.round(v * 10) / 10)
        ? String(Math.round(v))
        : v.toFixed(1)
      return `${text} ${unit}`
    }
  }
  return `${bytes} B`
}

// Compact "Jun 24, 14:02"-style stamp for the inferred-at moment. Falls back to
// a quiet placeholder when the epoch is missing (older cached records).
export function formatInferred(epochMs?: number | null): string {
  if (typeof epochMs !== 'number' || !Number.isFinite(epochMs)) {
    return 'unknown'
  }
  return new Date(epochMs).toLocaleString(undefined, {
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

/** "14:26" - the short form the inspector meta line uses. */
export function formatClockShort(epochMs?: number | null): string | null {
  if (typeof epochMs !== 'number' || !Number.isFinite(epochMs)) {
    return null
  }
  return new Date(epochMs).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}

/** Thousands-separated integer for file and entry counts. */
export function formatCount(n: number): string {
  return n.toLocaleString()
}
