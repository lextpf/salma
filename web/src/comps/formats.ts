// Archive format metadata. Tones are theme tokens, so tiles flip with dark
// mode. Rendering helpers live in FormatTile.tsx.
//
// This list is the engine's capability, not an aspiration. `format_of` in
// src/archive_service.rs routes by extension to exactly three readers and falls
// through to the zip reader for anything it does not name. Adding an extension
// here that no reader can open makes the picker accept a file that then fails at
// open, so add the reader first. A format's tone follows its reader: two
// extensions that share a reader share a colour, and the glyph tells them
// apart.

/** The reader that opens a format. `sidecar` is not an archive at all. */
export type Backend = 'sevenz_rust2' | 'unrar' | 'zip' | 'sidecar'

export interface FormatSpec {
  extension: string
  label: string
  /** Material Symbols ligature name, rendered through MIcon. */
  icon: string
  tone: string
  backend: Backend
}

export const FORMATS: FormatSpec[] = [
  { extension: '.7z', label: '7-Zip', icon: 'folder_zip', tone: 'var(--format-7z)', backend: 'sevenz_rust2' },
  // A split volume shares .7z's tone because it shares .7z's reader; the glyph
  // is what tells the two apart.
  { extension: '.001', label: 'Split volume', icon: 'layers', tone: 'var(--format-7z)', backend: 'sevenz_rust2' },
  { extension: '.rar', label: 'RAR', icon: 'deployed_code', tone: 'var(--format-rar)', backend: 'unrar' },
  { extension: '.zip', label: 'ZIP', icon: 'archive', tone: 'var(--format-zip)', backend: 'zip' },
  { extension: '.fomod', label: 'Installer', icon: 'extension', tone: 'var(--format-fomod)', backend: 'zip' },
  // Not an archive. The selections manifest, paired to an archive by filename
  // stem: useInstallation looks for `<stem>.json` beside the dropped file, and
  // the engine derives the same path when none is passed (resolve_json_path).
  { extension: '.json', label: 'Selections', icon: 'data_object', tone: 'var(--format-json)', backend: 'sidecar' },
]

/** A file the list above does not name. The engine will still try the zip reader. */
export const UNKNOWN: FormatSpec = {
  extension: 'file',
  label: 'Unknown',
  icon: 'draft',
  tone: 'var(--format-other)',
  backend: 'zip',
}

// Derived, not hand-written: the picker filter, the tiles and the count the
// intake rail prints are then provably the same list.
export const ACCEPT = FORMATS.map(f => f.extension).join(',')

export const SIDECAR = FORMATS.find(f => f.backend === 'sidecar')!

/**
 * The formats that are actually archives, in list order - everything the intake
 * rail shows at its extract stage. `backend` still separates them from the
 * sidecar; which reader opens which is engine detail the rail does not print.
 */
export const ARCHIVE_FORMATS: FormatSpec[] = FORMATS.filter(f => f.backend !== 'sidecar')

const BY_EXT: Record<string, FormatSpec> = Object.fromEntries(
  FORMATS.map(f => [f.extension.slice(1), f]),
)

/** Map a filename to its format spec by lowercased extension. */
export function formatForFile(fileName: string): FormatSpec {
  const ext = fileName.toLowerCase().split('.').pop() ?? ''
  return BY_EXT[ext] ?? UNKNOWN
}
