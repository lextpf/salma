/**
 * @brief Normalize supported cache schema variants before rendering.
 * @author Alex (<https://github.com/lextpf>)
 *
 * This adapter supplies display defaults. It does not validate replay input or nested diagnostics.
 */
import type { ConfidenceScore, FomodGroup, FomodReason, FomodStep } from './types'

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

/**
 * @fn asRecord(value: unknown): Record<string, unknown> | null
 * @brief Allow property access on non-null objects without validating their shape.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param value Untrusted cache value.
 * @return The object, including arrays, or null for other values.
 */
function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null
}

/**
 * @fn normalizePlugin(plugin: unknown, index: number, defaultSelected: boolean): NormalizedPlugin
 * @brief Adapt a cache value for plugin display.
 * @author Alex (<https://github.com/lextpf>)
 *
 * An explicit true selection flag takes precedence over a conflicting false flag.
 * Name aliases use the first non-null value; an empty name then uses the fallback.
 *
 * @param plugin Name, number, or object from a cache record.
 * @param index Zero-based position used for a fallback name.
 * @param defaultSelected Selection state when neither supported flag is boolean.
 * @return Display fields with optional confidence and reasons passed through.
 */
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

/**
 * @fn normalizePluginArray(value: unknown, defaultSelected: boolean): NormalizedPlugin[]
 * @brief Adapt plugin lists while tolerating absent or malformed containers.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param value Cache list to normalize.
 * @param defaultSelected Fallback selection state for each item.
 * @return Normalized items, or an empty list when the value is not an array.
 */
export function normalizePluginArray(value: unknown, defaultSelected: boolean): NormalizedPlugin[] {
  if (!Array.isArray(value)) return []
  return value.map((plugin, index) => normalizePlugin(plugin, index, defaultSelected))
}

/**
 * @fn normalizeGroups(step: FomodStep): NormalizedGroup[]
 * @brief Select the supported group layout for a cached step.
 * @author Alex (<https://github.com/lextpf>)
 *
 * A non-null optionalFileGroups value takes precedence over groups.
 * An absent, empty, or invalid chosen group list falls back to step-level plugins.
 * Group plugins default to selected; deselected and step-level plugins default to false.
 *
 * @param step Step that may use group arrays or a flat plugin list.
 * @return Groups in source order, or a synthetic group for a non-empty flat plugin list.
 */
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
