import type { InstallationJob } from '../types'
import ProgressBar from './ProgressBar'
import { Link } from 'react-router-dom'

interface InstallationQueueProps {
  jobs: InstallationJob[]
}

const statusConfig = {
  pending:    { label: 'Pending',    icon: 'fa-duotone fa-solid fa-clock',            color: 'outline' as const },
  uploading:  { label: 'Uploading',  icon: 'fa-duotone fa-solid fa-upload', color: 'info' as const },
  processing: { label: 'Installing', icon: 'fa-duotone fa-solid fa-gear fa-spin',      color: 'warning' as const },
  completed:  { label: 'Completed',  icon: 'fa-duotone fa-solid fa-circle-check',      color: 'success' as const },
  error:      { label: 'Error',      icon: 'fa-duotone fa-solid fa-circle-xmark',      color: 'error' as const },
} as const

const badgeClasses: Record<string, string> = {
  outline:  'bg-outline/10 text-outline border-outline/10',
  info:     'bg-info/10 text-info border-info/10',
  warning:  'bg-warning/10 text-warning border-warning/10',
  success:  'bg-success/10 text-success border-success/10',
  error:    'bg-error/10 text-error border-error/10',
}

export default function InstallationQueue({ jobs }: InstallationQueueProps) {
  if (jobs.length === 0) {
    return (
      <div className="animate-fade-in">
        <p className="ui-label text-info/80 mb-2">Queue</p>
        <div className="rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-5 flex items-center gap-3">
          <div className="w-9 h-9 rounded-xl bg-surface-container-high/60 border border-outline-variant/25 flex items-center justify-center shadow-elevation-1">
            <i className="fa-duotone fa-solid fa-inbox icon-gradient icon-gradient-steel text-[0.78rem]" />
          </div>
          <p className="text-sm text-on-surface-variant">
            Queue is empty. Drop archive files above to start an install.
          </p>
          <Link to="/logs" className="ml-auto ui-micro text-primary hover:underline underline-offset-2 whitespace-nowrap">
            View logs
          </Link>
        </div>
      </div>
    )
  }

  return (
    <div className="animate-fade-in">
      <div className="flex items-center gap-2 mb-2">
        <p className="ui-label text-info/80">Queue</p>
        <span className="text-[0.65rem] font-semibold text-on-surface-variant bg-surface-container-highest rounded-full px-1.5 py-0.5 tabular-nums">
          {jobs.length}
        </span>
      </div>
      <div className="flex flex-col gap-2.5">
        {jobs.map(job => {
          const { label, icon, color } = statusConfig[job.status]
          return (
            <div
              key={job.id}
              className="animate-slide-up rounded-xl aurora-card p-4 overflow-hidden relative
                shadow-elevation-1 transition-all duration-200 aurora-glow hover-intent"
            >
              <div className="flex items-center gap-3 mb-1">
                <i className={`${icon} text-[0.78rem] ${badgeClasses[color].split(' ').find(c => c.startsWith('text-'))}`} />
                <span className="font-medium text-sm text-on-surface truncate flex-1">
                  {job.fileName}
                </span>
                <span className={`shrink-0 rounded-full border px-2.5 py-0.5 text-[0.65rem] font-semibold ${badgeClasses[color]}`}>
                  {label}
                </span>
              </div>

              {(job.status === 'uploading' || job.status === 'processing') && (
                <ProgressBar
                  progress={job.uploadProgress || 0}
                  status={job.processingStatus || (job.status === 'uploading' ? 'Uploading file...' : 'Installing mod...')}
                />
              )}

              {job.modPath && (
                <div className="mt-2.5 rounded-lg bg-success/5 border border-success/15 px-3 py-2 text-[0.75rem] text-success-light flex items-center gap-2">
                  <i className="fa-duotone fa-solid fa-folder-open text-success text-[0.78rem]" />
                  <span className="truncate">{job.modPath}</span>
                </div>
              )}
              {job.error && (
                <div className="mt-2.5 rounded-lg bg-error/5 border border-error/15 px-3 py-2 text-[0.75rem] text-error-light flex items-center gap-2">
                  <i className="fa-duotone fa-solid fa-circle-exclamation text-error text-[0.78rem]" />
                  <span>{job.error}</span>
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
