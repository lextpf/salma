/**
 * @brief map accepted dashboard files to archive backends.
 * @author Alex (https://github.com/lextpf)
 *
 * keep accepted extensions synchronized with `archive_service.rs` readers.
 */
export type Backend = 'sevenz_rust2' | 'unrar' | 'zip' | 'sidecar'

export interface FormatSpec {
  extension: string
  label: string
  icon: string
  tone: string
  backend: Backend
}

// pair this sidecar with an archive by case-insensitive file stem.
const SIDECAR_SPEC: FormatSpec = {
  extension: '.json',
  label: 'Selections',
  icon: 'data_object',
  tone: 'var(--format-json)',
  backend: 'sidecar',
}

export const FORMATS: FormatSpec[] = [
  { extension: '.7z', label: '7-Zip', icon: 'folder_zip', tone: 'var(--format-7z)', backend: 'sevenz_rust2' },
  { extension: '.001', label: 'Split volume', icon: 'layers', tone: 'var(--format-7z)', backend: 'sevenz_rust2' },
  { extension: '.rar', label: 'RAR', icon: 'deployed_code', tone: 'var(--format-rar)', backend: 'unrar' },
  { extension: '.zip', label: 'ZIP', icon: 'archive', tone: 'var(--format-zip)', backend: 'zip' },
  { extension: '.fomod', label: 'Installer', icon: 'extension', tone: 'var(--format-fomod)', backend: 'zip' },
  SIDECAR_SPEC,
]

export const UNKNOWN: FormatSpec = {
  extension: 'file',
  label: 'Unknown',
  icon: 'draft',
  tone: 'var(--format-other)',
  backend: 'zip',
}

export const ACCEPT = FORMATS.map(f => f.extension).join(',')

export const SIDECAR = SIDECAR_SPEC

export const ARCHIVE_FORMATS: FormatSpec[] = FORMATS.filter(f => f.backend !== 'sidecar')

// a map, not an object: a file named `x.constructor` must miss, not reach Object.prototype.
const BY_EXT = new Map<string, FormatSpec>(FORMATS.map(f => [f.extension.slice(1), f]))

export function formatForFile(fileName: string): FormatSpec {
  const ext = fileName.toLowerCase().split('.').pop() ?? ''
  return BY_EXT.get(ext) ?? UNKNOWN
}
