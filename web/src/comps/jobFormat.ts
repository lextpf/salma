import type { InstallationJob } from '../types'

/**
 * The destination mod folder, as a leaf rather than a full path.
 *
 * `modName` arrives on the upload response, so it is known while the install is
 * still running. `modPath` lands only on completion and is absolute, so it is
 * reduced to its last segment; the full path overflows the meta line.
 */
export function modLeaf(job: InstallationJob): string | null {
  if (job.modName) return job.modName
  if (!job.modPath) return null
  const parts = job.modPath.replace(/[\\/]+$/, '').split(/[\\/]+/).filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : null
}

export function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

export function fmtClock(ms: number): string {
  const d = new Date(ms)
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
}
