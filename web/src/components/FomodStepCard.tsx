import type { FomodStep } from '../types'

interface FomodStepCardProps {
  step: FomodStep
  index: number
}

interface NormalizedPlugin {
  name: string
  selected: boolean
}

interface NormalizedGroup {
  name: string
  plugins: NormalizedPlugin[]
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null
}

function normalizePlugin(plugin: unknown, index: number): NormalizedPlugin {
  if (typeof plugin === 'string') return { name: plugin, selected: false }
  if (typeof plugin === 'number') return { name: String(plugin), selected: false }

  const rec = asRecord(plugin)
  if (!rec) return { name: `Plugin ${index + 1}`, selected: false }

  const rawName = rec.name ?? rec.pluginName ?? rec.displayName ?? rec.file
  const name = typeof rawName === 'string' && rawName.trim().length > 0
    ? rawName
    : `Plugin ${index + 1}`

  return {
    name,
    selected: rec.selected === true || rec.isSelected === true,
  }
}

function normalizePlugins(value: unknown): NormalizedPlugin[] {
  if (!Array.isArray(value)) return []
  return value.map((plugin, index) => normalizePlugin(plugin, index))
}

function normalizeGroups(step: FomodStep): NormalizedGroup[] {
  const rawGroups = step.optionalFileGroups ?? step.groups
  if (Array.isArray(rawGroups) && rawGroups.length > 0) {
    return rawGroups.map((group, gi) => {
      const rec = asRecord(group)
      const rawGroupName = rec?.name
      const name = typeof rawGroupName === 'string' && rawGroupName.trim().length > 0
        ? rawGroupName
        : `Group ${gi + 1}`
      const plugins = normalizePlugins(rec?.plugins)
      return { name, plugins }
    })
  }

  const stepPlugins = normalizePlugins(step.plugins)
  if (stepPlugins.length > 0) {
    return [{ name: 'Plugins', plugins: stepPlugins }]
  }

  return []
}

export default function FomodStepCard({ step, index }: FomodStepCardProps) {
  const name = step.name || `Step ${index + 1}`
  const groups = normalizeGroups(step)
  const stepNum = String(index + 1).padStart(2, '0')

  return (
    <div
      style={{
        background: 'var(--card)',
        border: '1px solid var(--rule)',
        borderRadius: 'var(--radius-md)',
        padding: '20px 24px',
      }}
    >
      <div className="flex items-baseline" style={{ gap: 12, marginBottom: 14 }}>
        <span
          className="display-serif-italic"
          style={{
            fontSize: 18,
            color: 'var(--accent)',
            letterSpacing: '-0.01em',
            lineHeight: 1,
          }}
        >
          {stepNum}
        </span>
        <span
          aria-hidden="true"
          style={{
            width: 18,
            height: 1,
            background: 'var(--rule-strong)',
            opacity: 0.55,
            transform: 'translateY(-4px)',
          }}
        />
        <h3
          className="display-serif-tight"
          style={{ fontSize: 18, color: 'var(--ink)', letterSpacing: '-0.01em' }}
        >
          {name}
        </h3>
      </div>

      {groups.length > 0 ? (
        <div className="flex flex-col" style={{ gap: 12 }}>
          {groups.map((group, gi) => {
            const groupName = group.name || `Group ${gi + 1}`
            const plugins = group.plugins || []
            const selectedPlugin = plugins.find(p => p.selected === true)
            return (
              <div
                key={gi}
                style={{
                  background: 'var(--paper-2)',
                  border: '1px solid var(--rule-soft)',
                  borderRadius: 'var(--radius-sm)',
                  padding: '12px 16px',
                }}
              >
                <p className="ui-label" style={{ marginBottom: 8, fontSize: 9.5 }}>
                  {groupName}
                </p>
                {selectedPlugin ? (
                  <p
                    className="flex items-start"
                    style={{ gap: 8, fontSize: 13, color: 'var(--ink)' }}
                  >
                    <span className="dot-status dot-status-on" style={{ marginTop: 6 }} />
                    <span>{selectedPlugin.name || 'Unknown'}</span>
                  </p>
                ) : (
                  <ul className="flex flex-col" style={{ gap: 4, fontSize: 13 }}>
                    {plugins.slice(0, 8).map((plugin, pi) => (
                      <li
                        key={pi}
                        className="flex items-start"
                        style={{
                          gap: 8,
                          color: plugin.selected ? 'var(--ink)' : 'var(--ink-3)',
                        }}
                      >
                        <span
                          className={`dot-status ${plugin.selected ? 'dot-status-on' : ''}`}
                          style={{ marginTop: 6 }}
                        />
                        <span>{plugin.name || `Plugin ${pi + 1}`}</span>
                      </li>
                    ))}
                    {plugins.length > 8 && (
                      <li
                        className="timestamp-print"
                        style={{ marginTop: 4, paddingLeft: 14 }}
                      >
                        // ... and {plugins.length - 8} more
                      </li>
                    )}
                  </ul>
                )}
              </div>
            )
          })}
        </div>
      ) : (
        <p className="timestamp-print">
          // no groups in this step
        </p>
      )}
    </div>
  )
}
