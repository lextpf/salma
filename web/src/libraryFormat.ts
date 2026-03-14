// Shared formatting helpers for the Library triptych (records list, inspector
// header, files tab). Kept out of the .tsx component files so the screen's
// components stay single-export and fast-refresh friendly.

export function formatSize(bytes?: number): string {
  if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes < 0) {
    return 'n/a'
  }
  if (bytes < 1024) return `${bytes} b`
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} kb`
  return `${(bytes / 1048576).toFixed(1)} mb`
}

// Compact "Jun 24 - 14:02"-style stamp for the inferred-at moment. Falls back to
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
