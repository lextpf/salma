interface FomodStepCardProps {
  step: Record<string, unknown>
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
  if (typeof plugin === 'string') {
    return { name: plugin, selected: false }
  }
  if (typeof plugin === 'number') {
    return { name: String(plugin), selected: false }
  }

  const rec = asRecord(plugin)
  if (!rec) {
    return { name: `Plugin ${index + 1}`, selected: false }
  }

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

function normalizeGroups(step: Record<string, unknown>): NormalizedGroup[] {
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

  // Some FOMOD JSONs store plugins directly under step.plugins.
  const stepPlugins = normalizePlugins(step.plugins)
  if (stepPlugins.length > 0) {
    return [{ name: 'Plugins', plugins: stepPlugins }]
  }

  return []
}

export default function FomodStepCard({ step, index }: FomodStepCardProps) {
  const name = (step.name as string) || `Step ${index + 1}`
  const groups = normalizeGroups(step)

  return (
    <div className="relative rounded-2xl aurora-card p-5 transition-all duration-200 aurora-glow overflow-hidden group">
      {/* Top accent gradient */}
      <div className="absolute top-0 left-6 right-6 h-px bg-gradient-to-r from-transparent via-primary/15 to-transparent" />

      <h3 className="relative text-base font-semibold text-on-surface mb-3 flex items-center gap-3">
        <span className="w-7 h-7 rounded-lg bg-gradient-to-br from-primary/20 to-secondary/10 border border-outline-variant/20 flex items-center justify-center text-xs font-bold text-primary tabular-nums shadow-[0_0_10px_-3px_rgba(56,189,248,0.15)]">
          {index + 1}
        </span>
        {name}
      </h3>
      {groups.length > 0 ? (
        <div className="relative space-y-2.5 ml-10">
          {groups.map((group, gi) => {
            const groupName = group.name || `Group ${gi + 1}`
            const plugins = group.plugins || []
            const selectedPlugin = plugins.find(p => p.selected === true)
            return (
              <div key={gi} className="rounded-xl bg-surface-container-high/60 p-3 border border-outline-variant/20 backdrop-blur-sm">
                <p className="text-[0.82rem] font-medium text-on-surface-variant mb-1.5 flex items-start gap-1.5 leading-snug">
                  <span className="pt-[1px]">
                    <i className="fa-duotone fa-solid fa-puzzle-piece text-[0.62rem] icon-gradient icon-gradient-aurora" />
                  </span>
                  {groupName}
                </p>
                {selectedPlugin ? (
                  <p className="text-[0.82rem] text-success-light flex items-start gap-1.5 leading-snug">
                    <span className="text-success pt-[1px]"><i className="fa-duotone fa-solid fa-circle-check text-[0.64rem]" /></span>
                    {selectedPlugin.name || 'Unknown'}
                  </p>
                ) : (
                  <ul className="text-[0.82rem] text-on-surface-variant leading-snug space-y-0.5">
                    {plugins.slice(0, 5).map((plugin, pi) => (
                      <li key={pi} className="flex items-start gap-1.5">
                        {plugin.selected === true
                          ? <span className="text-success pt-[1px]"><i className="fa-duotone fa-solid fa-circle-check text-[0.62rem]" /></span>
                          : <span className="text-outline pt-[1px]"><i className="fa-duotone fa-solid fa-circle text-[0.62rem]" /></span>
                        }
                        <span className={plugin.selected === true ? 'text-success-light' : ''}>
                          {plugin.name || `Plugin ${pi + 1}`}
                        </span>
                      </li>
                    ))}
                    {plugins.length > 5 && (
                      <li className="text-outline text-xs ml-4">... and {plugins.length - 5} more</li>
                    )}
                  </ul>
                )}
              </div>
            )
          })}
        </div>
      ) : (
        <p className="relative text-sm text-on-surface-variant ml-10">
          <span className="text-outline"><i className="fa-duotone fa-solid fa-circle-info mr-1.5 text-xs" /></span>No groups in this step
        </p>
      )}
    </div>
  )
}
