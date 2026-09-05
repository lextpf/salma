import type { InstallationJob } from '../types'

/**
 * @fn modLeaf(job: InstallationJob): string | null
 * @brief select a compact destination name for job rows.
 * @author Alex (https://github.com/lextpf)
 *
 * prefer the upload-time name. completed jobs can use the last path segment.
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
