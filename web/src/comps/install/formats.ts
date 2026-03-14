// Archive format metadata, salvaged from the retired Dropzone. Tones are theme
// tokens so tiles flip with dark mode. Rendering helpers live in FormatTile.tsx.

export interface FormatSpec {
  extension: string
  label: string
  icon: string
  tone: string
}

export const FORMATS: FormatSpec[] = [
  { extension: '.7z', label: '7-Zip', icon: 'fa-file-zipper', tone: 'var(--format-7z)' },
  { extension: '.zip', label: 'ZIP', icon: 'fa-box-archive', tone: 'var(--format-zip)' },
  { extension: '.rar', label: 'RAR', icon: 'fa-layer-group', tone: 'var(--format-rar)' },
  { extension: '.fomod', label: 'Installer', icon: 'fa-puzzle-piece', tone: 'var(--format-fomod)' },
  { extension: '.json', label: 'Manifest', icon: 'fa-code', tone: 'var(--format-json)' },
  { extension: '.tar*', label: 'Unix', icon: 'fa-cubes-stacked', tone: 'var(--format-tar)' },
]

export const ACCEPT = '.001,.7z,.fomod,.zip,.rar,.tar,.gz,.bz2,.xz,.json'

const BY_EXT: Record<string, FormatSpec> = {
  '7z': FORMATS[0],
  zip: FORMATS[1],
  rar: FORMATS[2],
  fomod: FORMATS[3],
  json: FORMATS[4],
  tar: FORMATS[5],
  gz: FORMATS[5],
  bz2: FORMATS[5],
  xz: FORMATS[5],
  '001': FORMATS[5],
}

// Map a filename to its format spec by lowercased extension. Unknown or unlisted
// archive segments fall back to the tar* tone so every row still gets a tile.
export function formatForFile(fileName: string): FormatSpec {
  const ext = fileName.toLowerCase().split('.').pop() ?? ''
  return BY_EXT[ext] ?? FORMATS[5]
}
