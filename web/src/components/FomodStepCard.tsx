import type { ConfidenceScore, FomodGroup, FomodReason, FomodStep } from '../types'
import ConfidencePill from './ConfidencePill'
import WhyPanel from './WhyPanel'

interface FomodStepCardProps {
  step: FomodStep
  index: number
}

interface NormalizedPlugin {
  name: string
  selected: boolean
  confidence?: ConfidenceScore
  reasons?: FomodReason[]
}

interface NormalizedGroup {
  name: string
  plugins: NormalizedPlugin[]
  deselected: NormalizedPlugin[]
  confidence?: ConfidenceScore
  resolved_by?: string
  reasons?: FomodReason[]
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null
}

function normalizePlugin(
  plugin: unknown,
  index: number,
  defaultSelected: boolean,
): NormalizedPlugin {
  if (typeof plugin === 'string') return { name: plugin, selected: defaultSelected }
  if (typeof plugin === 'number') return { name: String(plugin), selected: defaultSelected }

  const rec = asRecord(plugin)
  if (!rec) return { name: `Plugin ${index + 1}`, selected: defaultSelected }

  const rawName = rec.name ?? rec.pluginName ?? rec.displayName ?? rec.file
  const name =
    typeof rawName === 'string' && rawName.trim().length > 0 ? rawName : `Plugin ${index + 1}`

  const selected =
    rec.selected === true || rec.isSelected === true
      ? true
      : rec.selected === false || rec.isSelected === false
        ? false
        : defaultSelected

  const confidence = rec.confidence as ConfidenceScore | undefined
  const reasons = Array.isArray(rec.reasons) ? (rec.reasons as FomodReason[]) : undefined

  return { name, selected, confidence, reasons }
}

function normalizePluginArray(value: unknown, defaultSelected: boolean): NormalizedPlugin[] {
  if (!Array.isArray(value)) return []
  return value.map((plugin, index) => normalizePlugin(plugin, index, defaultSelected))
}

function normalizeGroups(step: FomodStep): NormalizedGroup[] {
  const rawGroups = step.optionalFileGroups ?? step.groups
  if (Array.isArray(rawGroups) && rawGroups.length > 0) {
    return rawGroups.map((group, gi) => {
      const rec = asRecord(group)
      const rawGroupName = rec?.name
      const name =
        typeof rawGroupName === 'string' && rawGroupName.trim().length > 0
          ? rawGroupName
          : `Group ${gi + 1}`
      const plugins = normalizePluginArray(rec?.plugins, true)
      const deselected = normalizePluginArray(rec?.deselected, false)
      const confidence = (rec as FomodGroup | null)?.confidence
      const resolved_by = typeof rec?.resolved_by === 'string' ? rec.resolved_by : undefined
      const reasons = Array.isArray(rec?.reasons) ? (rec.reasons as FomodReason[]) : undefined
      return { name, plugins, deselected, confidence, resolved_by, reasons }
    })
  }

  const stepPlugins = normalizePluginArray(step.plugins, false)
  if (stepPlugins.length > 0) {
    return [{ name: 'Plugins', plugins: stepPlugins, deselected: [] }]
  }

  return []
}

function PluginRow({ plugin }: { plugin: NormalizedPlugin }) {
  return (
    <li className="flex flex-col" style={{ gap: 4 }}>
      <div
        className="flex items-start"
        style={{ gap: 8, color: plugin.selected ? 'var(--ink)' : 'var(--ink-3)' }}
      >
        <span
          className={`dot-status ${plugin.selected ? 'dot-status-on' : ''}`}
          style={{ marginTop: 6 }}
        />
        <span style={{ flex: 1, fontSize: 14.5 }}>{plugin.name}</span>
        {plugin.confidence && <ConfidencePill confidence={plugin.confidence} size="sm" />}
      </div>
      {(plugin.reasons?.length || plugin.confidence) && (
        <div style={{ paddingLeft: 18 }}>
          <WhyPanel confidence={plugin.confidence} reasons={plugin.reasons} />
        </div>
      )}
    </li>
  )
}

function ResolvedByChip({ resolved_by }: { resolved_by?: string }) {
  if (!resolved_by) return null
  return (
    <span
      className="flex items-center"
      style={{
        gap: 6,
        padding: '2px 8px',
        borderRadius: 3,
        border: '1px solid var(--rule-soft)',
        background: 'var(--paper-3)',
        fontFamily: 'var(--font-mono)',
        fontSize: 10,
        letterSpacing: '0.12em',
        textTransform: 'uppercase',
        color: 'var(--ink-3)',
      }}
      title={`Group resolved by ${resolved_by}`}
    >
      <i className="fa-duotone fa-solid fa-link" style={{ fontSize: 9 }} />
      <span>{resolved_by}</span>
    </span>
  )
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
      <div className="flex items-baseline" style={{ gap: 12, marginBottom: 14, flexWrap: 'wrap' }}>
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
          style={{ fontSize: 18, color: 'var(--ink)', letterSpacing: '-0.01em', flex: 1 }}
        >
          {name}
        </h3>
        {step.confidence && <ConfidencePill confidence={step.confidence} size="md" />}
      </div>
      {step.reasons && step.reasons.length > 0 && (
        <div style={{ marginBottom: 12 }}>
          <WhyPanel reasons={step.reasons} title="Step details" />
        </div>
      )}

      {groups.length > 0 ? (
        <div className="flex flex-col" style={{ gap: 12 }}>
          {groups.map((group, gi) => {
            const groupName = group.name || `Group ${gi + 1}`
            const allPlugins = [...group.plugins, ...group.deselected]
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
                <div
                  className="flex items-center"
                  style={{ gap: 10, marginBottom: 10, flexWrap: 'wrap' }}
                >
                  <p className="ui-label" style={{ margin: 0, fontSize: 11, flex: 1 }}>
                    {groupName}
                  </p>
                  <ResolvedByChip resolved_by={group.resolved_by} />
                  {group.confidence && (
                    <ConfidencePill confidence={group.confidence} size="sm" />
                  )}
                </div>
                {allPlugins.length === 0 ? (
                  <p className="timestamp-print">// no plugins recorded</p>
                ) : (
                  <ul className="flex flex-col" style={{ gap: 8 }}>
                    {allPlugins.map((p, pi) => (
                      <PluginRow key={pi} plugin={p} />
                    ))}
                  </ul>
                )}
              </div>
            )
          })}
        </div>
      ) : (
        <p className="timestamp-print">// no groups in this step</p>
      )}
    </div>
  )
}
