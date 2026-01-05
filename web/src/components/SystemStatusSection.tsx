import { Link } from 'react-router-dom'
import type { Mo2Status, AppConfig } from '../types'
import type { ReadinessStyles } from '../utils/readinessStyles'

interface SystemStatusSectionProps {
  status: Mo2Status | null
  config: AppConfig | null
  readiness: number | null
  styles: ReadinessStyles
  ringPulse: boolean
  systemExpanded: boolean
  setSystemExpanded: (expanded: boolean) => void
  statusUpdatedAt: number | null
  relativeTimeNode: React.ReactNode
}

export default function SystemStatusSection({
  status,
  config,
  styles,
  ringPulse,
  systemExpanded,
  setSystemExpanded,
  relativeTimeNode,
}: SystemStatusSectionProps) {
  if (!systemExpanded) return null

  if (!status || !config) {
    return (
      <>
        <div className="mb-8 rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-5">
          <div className="skeleton-line h-4 w-40 mb-4" />
          <div className="skeleton-line h-3 w-64 mb-2" />
          <div className="skeleton-line h-3 w-52" />
        </div>
        {/* Aurora divider */}
        <div className="aurora-divider mb-8" />
      </>
    )
  }

  const healthChecks = [
    {
      label: 'MO2 Plugin',
      icon: 'fa-duotone fa-solid fa-puzzle-piece',
      ok: status.pluginInstalled,
      goodValue: 'Installed',
      badValue: 'Missing',
      detail: status.pluginDeployPath || 'No deploy path',
      grad: 'icon-gradient-spring',
    },
    {
      label: 'Configuration',
      icon: 'fa-duotone fa-solid fa-plug-circle-check',
      ok: status.configured,
      goodValue: 'Active',
      badValue: 'Not set',
      detail: config.mo2ModsPath || 'No mods path set',
      grad: 'icon-gradient-atlas',
    },
    {
      label: 'FOMOD Output',
      icon: 'fa-duotone fa-solid fa-folder-open',
      ok: status.outputFolderExists,
      goodValue: 'Ready',
      badValue: 'Missing',
      detail: status.fomodOutputDir || 'N/A',
      grad: 'icon-gradient-ocean',
    },
  ]

  const readyCount = healthChecks.filter(h => h.ok).length

  return (
    <>
      <div className="mb-8">
        {!status.configured && (
          <div className="mb-6 rounded-2xl border border-warning/15 bg-warning/5 p-5 flex items-start gap-4 animate-slide-up">
            <div className="w-9 h-9 rounded-xl bg-warning/12 flex items-center justify-center shrink-0">
              <i className="fa-duotone fa-solid fa-triangle-exclamation text-warning" />
            </div>
            <div>
              <p className="text-warning font-semibold text-sm">MO2 paths not configured</p>
              <p className="text-on-surface-variant text-xs mt-0.5">
                Go to <Link to="/settings" className="text-warning underline underline-offset-2 font-medium hover:text-warning-light">Settings</Link> to set your MO2 mods path.
              </p>
            </div>
          </div>
        )}

        <div className="flex items-start gap-4 mb-8">
          <div className="shrink-0 flex items-center gap-4">
            <button
              onClick={() => setSystemExpanded(false)}
              className="readiness-glow relative w-[5.5rem] h-[5.5rem] rounded-full hover:scale-105 transition-transform"
              style={{ '--readiness-glow-color': styles.readinessGlowColor } as React.CSSProperties}
              title="Click to collapse system section"
            >
              <div
                className={`absolute inset-0 rounded-full ${ringPulse ? 'ring-pulse-once' : ''}`}
                style={{
                  background: styles.ringBackground,
                  WebkitMaskImage: 'radial-gradient(farthest-side, transparent calc(100% - 7px), #000 calc(100% - 7px))',
                  maskImage: 'radial-gradient(farthest-side, transparent calc(100% - 7px), #000 calc(100% - 7px))',
                }}
              />
              <div className="absolute inset-[7px] rounded-full bg-surface-container border border-outline-variant/35" />
              <div className="absolute inset-0 flex flex-col items-center justify-center">
                <p className={`text-[0.9rem] font-bold tabular-nums leading-none ${styles.readinessTextClass}`}>{styles.readinessValue}%</p>
                <p className={`text-[0.55rem] uppercase tracking-[0.11em] mt-0.5 ${styles.readinessLabelClass}`}>Readiness</p>
              </div>
            </button>

            <div>
              <p className="ui-micro uppercase tracking-[0.11em]">System</p>
              <p className="text-xs font-semibold text-on-surface mt-1">
                {readyCount} of {healthChecks.length} checks
              </p>
              <p className="ui-micro mt-1">
                {relativeTimeNode}
              </p>
            </div>
          </div>

          <div className="flex flex-wrap gap-x-6 gap-y-1 pt-0.5">
            <div>
              <p className="ui-label text-primary/85 mb-1.5">Health Checks</p>
              <div className="flex flex-wrap gap-1.5">
                {healthChecks.map(item => (
                  <div
                    key={item.label}
                    title={`${item.label}: ${item.ok ? item.goodValue : item.badValue} - ${item.detail}`}
                    className="rounded-full px-2.5 py-1 bg-transparent inline-flex items-center gap-1.5 hover-intent"
                  >
                    <span className="inline-flex items-center justify-center w-4">
                      <i className={`${item.icon} icon-gradient ${item.grad} icon-sm`} />
                    </span>
                    <span className="text-[0.68rem] font-semibold uppercase tracking-[0.08em] text-on-surface-variant">
                      {item.label}
                    </span>
                    <span
                      className={`w-2 h-2 rounded-full ${
                        item.ok
                          ? 'bg-success shadow-[0_0_8px_rgba(34,197,94,0.55)]'
                          : 'bg-warning shadow-[0_0_8px_rgba(245,158,11,0.55)]'
                      }`}
                      aria-hidden="true"
                    />
                  </div>
                ))}
              </div>
            </div>

            <div>
              <p className="ui-label text-secondary/85 mb-1.5">Library Metrics</p>
              <div className="flex flex-wrap gap-1.5">
                <div
                  title={status.fomodOutputDir || 'N/A'}
                  className="rounded-full px-2.5 py-1 bg-transparent inline-flex items-center gap-1.5 hover-intent"
                >
                  <i className="fa-duotone fa-solid fa-file-code icon-gradient icon-gradient-atlas icon-sm" />
                  <span className="text-[0.68rem] font-semibold uppercase tracking-[0.08em] text-on-surface-variant">JSONs</span>
                  <span className="text-[0.74rem] font-bold tabular-nums text-primary metric-value-glow-primary">{status.jsonCount}</span>
                </div>

                <div
                  title={config.mo2ModsPath || 'No mods path set'}
                  className="rounded-full px-2.5 py-1 bg-transparent inline-flex items-center gap-1.5 hover-intent"
                >
                  <i className="fa-duotone fa-solid fa-boxes-stacked icon-gradient icon-gradient-forest icon-sm" />
                  <span className="text-[0.68rem] font-semibold uppercase tracking-[0.08em] text-on-surface-variant">Folders</span>
                  <span className="text-[0.74rem] font-bold tabular-nums text-secondary metric-value-glow-secondary">{status.modCount}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
      {/* Aurora divider */}
      <div className="aurora-divider mb-8" />
    </>
  )
}
