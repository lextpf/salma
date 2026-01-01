interface ProgressBarProps {
  progress: number // 0-100
  status?: string
}

export default function ProgressBar({ progress, status }: ProgressBarProps) {
  const clamped = Math.min(100, Math.max(0, progress))
  return (
    <div className="w-full mt-3">
      <div className="w-full h-1.5 bg-surface-container-highest rounded-full overflow-hidden">
        <div
          className="h-full rounded-full bg-gradient-to-r from-primary via-secondary to-tertiary transition-all duration-500 ease-out shadow-[0_0_8px_-2px_rgba(56,189,248,0.4)]"
          style={{ width: `${clamped}%` }}
        />
      </div>
      <div className="flex justify-between mt-1.5 text-[0.75rem] text-on-surface-variant">
        {status && <span className="flex-1 truncate">{status}</span>}
        <span className="font-semibold min-w-[40px] text-right tabular-nums">{Math.round(clamped)}%</span>
      </div>
    </div>
  )
}
