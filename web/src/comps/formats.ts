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

export const FORMATS: FormatSpec[] = [
  { extension: '.7z', label: '7-Zip', icon: 'folder_zip', tone: 'var(--format-7z)', backend: 'sevenz_rust2' },
  { extension: '.001', label: 'Split volume', icon: 'layers', tone: 'var(--format-7z)', backend: 'sevenz_rust2' },
  { extension: '.rar', label: 'RAR', icon: 'deployed_code', tone: 'var(--format-rar)', backend: 'unrar' },
  { extension: '.zip', label: 'ZIP', icon: 'archive', tone: 'var(--format-zip)', backend: 'zip' },
  { extension: '.fomod', label: 'Installer', icon: 'extension', tone: 'var(--format-fomod)', backend: 'zip' },
  // pair this sidecar with an archive by case-insensitive file stem.
  { extension: '.json', label: 'Selections', icon: 'data_object', tone: 'var(--format-json)', backend: 'sidecar' },
]

export const UNKNOWN: FormatSpec = {
  extension: 'file',
  label: 'Unknown',
  icon: 'draft',
  tone: 'var(--format-other)',
  backend: 'zip',
}

export const ACCEPT = FORMATS.map(f => f.extension).join(',')

export const SIDECAR = FORMATS.find(f => f.backend === 'sidecar')!

export const ARCHIVE_FORMATS: FormatSpec[] = FORMATS.filter(f => f.backend !== 'sidecar')

const BY_EXT: Record<string, FormatSpec> = Object.fromEntries(
  FORMATS.map(f => [f.extension.slice(1), f]),
)

export function formatForFile(fileName: string): FormatSpec {
  const ext = fileName.toLowerCase().split('.').pop() ?? ''
  return BY_EXT[ext] ?? UNKNOWN
}
