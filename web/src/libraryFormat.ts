/**
 * @fn formatSize(bytes?: number): string
 * @brief produce compact binary-unit text for table cells.
 * @author Alex (https://github.com/lextpf)
 *
 * values below 1024 use bytes. scaled values use at most one decimal place.
 * @param bytes a non-negative byte count, or undefined.
 * @return the formatted count, or `n/a` for invalid input.
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
      const text = v >= 100 || Number.isInteger(Math.round(v * 10) / 10)
        ? String(Math.round(v))
        : v.toFixed(1)
      return `${text} ${unit}`
    }
  }
  return `${bytes} B`
}

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

export function formatCount(n: number): string {
  return n.toLocaleString()
}
