import { Link } from 'react-router-dom'
import type { FomodEntry } from '../types'

interface FomodListItemProps {
  fomod: FomodEntry
  onDelete: (name: string) => void
  disabled?: boolean
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1048576).toFixed(1)} MB`
}

function formatDate(epochMs: number): string {
  return new Date(epochMs).toLocaleDateString(undefined, {
    year: 'numeric', month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit',
  })
}

export default function FomodListItem({ fomod, onDelete, disabled }: FomodListItemProps) {
  return (
    <div className="group flex items-center gap-4 rounded-xl bg-surface-container p-4
                    transition-colors duration-150 hover:bg-surface-container-high fomod-entry-glow hover-intent">
      {/* Icon */}
      <div className="shrink-0 w-10 h-10 rounded-xl bg-gradient-to-br from-primary/12 to-secondary/8 border border-outline-variant/20 flex items-center justify-center
                      transition-transform duration-150 group-hover:scale-105">
        <i className="fa-duotone fa-solid fa-file-code icon-gradient icon-gradient-orchid" />
      </div>

      {/* Content */}
      <div className="flex-1 min-w-0">
        <Link
          to={`/fomods/${encodeURIComponent(fomod.name)}`}
          className="text-sm font-medium text-on-surface hover:text-primary-light transition-colors truncate block"
        >
          {fomod.name}
        </Link>
        <div className="flex gap-4 mt-1 ui-micro">
          <span><i className="fa-duotone fa-solid fa-layer-group mr-1 icon-gradient icon-gradient-atlas fomod-meta-icon icon-sm" />{fomod.stepCount} {fomod.stepCount === 1 ? 'step' : 'steps'}</span>
          <span><i className="fa-duotone fa-solid fa-weight-hanging mr-1 icon-gradient icon-gradient-copper fomod-meta-icon icon-sm" />{formatSize(fomod.size)}</span>
          <span className="hidden sm:inline"><i className="fa-duotone fa-solid fa-clock mr-1 icon-gradient icon-gradient-mint fomod-meta-icon fomod-meta-clock icon-sm" />{formatDate(fomod.modified)}</span>
        </div>
      </div>

      {/* Actions */}
      <button
        onClick={() => onDelete(fomod.name)}
        disabled={disabled}
        className={`shrink-0 w-8 h-8 rounded-lg flex items-center justify-center transition-colors opacity-0 group-hover:opacity-100
                   ${disabled ? 'text-outline/40 cursor-not-allowed' : 'text-error/60 bg-transparent hover:bg-error/10 hover:text-error'}`}
        title="Delete"
      >
        <i className="fa-duotone fa-solid fa-trash-can text-xs" />
      </button>

      <Link
        to={`/fomods/${encodeURIComponent(fomod.name)}`}
        className="shrink-0 w-8 h-8 rounded-lg flex items-center justify-center text-on-surface-variant
                   hover:bg-primary/10 hover:text-primary transition-colors"
        title="View"
      >
        <i className="fa-duotone fa-solid fa-chevron-right text-xs" />
      </Link>
    </div>
  )
}
