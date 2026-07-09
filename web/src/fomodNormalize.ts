import type { ConfidenceScore, FomodGroup, FomodReason, FomodStep } from './types'

// Defensive normalization of the schema-v2 step/group/plugin shapes into a
// stable view model. The inferred JSON has drifted across schema versions
// (plugins may be strings, numbers, or objects; groups live under
// `optionalFileGroups` or `groups`; selection lives in `selected`/`isSelected`),
// so every consumer reads through these helpers. Extracted from the old
// FomodStepCard so the Library StepsTab and any future surface share one parser.

export interface NormalizedPlugin {
  name: string
  selected: boolean
  confidence?: ConfidenceScore
  reasons?: FomodReason[]
}

export interface NormalizedGroup {
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

export function normalizePlugin(
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

export function normalizePluginArray(value: unknown, defaultSelected: boolean): NormalizedPlugin[] {
  if (!Array.isArray(value)) return []
  return value.map((plugin, index) => normalizePlugin(plugin, index, defaultSelected))
}

export function normalizeGroups(step: FomodStep): NormalizedGroup[] {
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
