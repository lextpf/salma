/**
 * @interface InstallationJob
 * @brief represent browser-owned state for one sequential install.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-transit-connection-variant: state transitions
 *
 * @verbatim
 * pending -> uploading -> processing -> completed
 *    |          |              |
 *    +----------+--------------+----------> error
 * @endverbatim
 *
 * jobs run sequentially. cancellation uses the terminal `error` state.
 *
 * ### :material-shield-check: state invariants
 *
 * `modName` is present after upload. `modPath` is present after completion.
 * `error` is present only in the error state. `processingStatus` is display-only.
 * upload progress is in [0, 100]. browser timestamps are Unix epoch milliseconds.
 */
export interface InstallationJob {
  id: string
  fileName: string
  status: 'pending' | 'uploading' | 'processing' | 'completed' | 'error'
  modPath?: string
  modName?: string
  error?: string
  uploadProgress?: number
  processingStatus?: string
  createdAt: number
  completedAt?: number
  sizeBytes?: number
}

export interface AppConfig {
  mo2ModsPath: string
  fomodOutputDir: string
  mo2ModsPathValid: boolean
}

export interface Mo2Status {
  configured: boolean
  outputFolderExists: boolean
  fomodOutputDir: string
  jsonCount: number
  modCount: number
  pluginInstalled: boolean
  pluginDeployPath: string
}

/**
 * @interface FomodEntry
 * @brief expose cache-file metadata before detail loading.
 * @author Alex (https://github.com/lextpf)
 *
 * `size` and `modified` describe the cached JSON file, not the mod.
 * the server can return undeclared `parseError`. do not trust parsed fields without handling it.
 * `size` is in bytes. `modified` is Unix epoch milliseconds.
 * `confidence` is in [0, 1] and is absent when diagnostics are unavailable.
 */
export interface FomodEntry {
  name: string
  size: number
  modified: number
  stepCount: number
  confidence?: number
  confidenceBand?: ConfidenceBand
  exactMatch?: boolean
}

export interface LogsResponse {
  lines: string[]
  errors: number
  warnings: number
  passes: number
  nextOffset?: number
  reset?: boolean
}

export interface TestStatus {
  running: boolean
  exitCode?: number
  pid?: number
}

export interface FomodScanResult {
  success: boolean
  totalModFolders: number
  archivesProcessed: number
  choicesInferred: number
  noFomod: number
  alreadyHadChoices: number
  noArchiveFound: number
  archiveMissing: number
  errors: number
  durationMs: number
  outputDir: string
}

export interface FomodScanStatus {
  running: boolean
  success?: boolean
  error?: string
  totalModFolders?: number
  archivesProcessed?: number
  choicesInferred?: number
  noFomod?: number
  alreadyHadChoices?: number
  noArchiveFound?: number
  archiveMissing?: number
  errors?: number
  durationMs?: number
  outputDir?: string
}

export interface PluginActionResult {
  started: boolean
  action: string
}

export interface PluginActionStatus {
  running: boolean
  success?: boolean
  exitCode?: number
  pluginInstalled?: boolean
  pluginDeployPath?: string
  action?: string
  error?: string
}

export interface InstallStatus {
  running: boolean
  success?: boolean
  modPath?: string
  modName?: string
  error?: string
}

/**
 * @brief represent serialized confidence bands from the engine.
 * @author Alex (https://github.com/lextpf)
 *
 * dashboard tiers use separate thresholds in `confidence.ts`.
 */
export type ConfidenceBand = 'high' | 'medium' | 'low'

/**
 * @interface ConfidenceComponents
 * @brief hold normalized inputs to the weighted confidence score.
 * @author Alex (https://github.com/lextpf)
 *
 * values are in [0, 1]. they are not percentages. some components are grades.
 */
export interface ConfidenceComponents {
  evidence: number
  propagation: number
  repro: number
  ambiguity: number
}

/**
 * @interface ConfidenceScore
 * @brief report the weighted confidence result and engine band.
 * @author Alex (https://github.com/lextpf)
 *
 * `composite` is a clamped value in [0, 1]. it is not an unweighted
 * component average.
 */
export interface ConfidenceScore {
  composite: number
  band: ConfidenceBand
  components: ConfidenceComponents
}

export interface FomodReason {
  code: string
  message: string
  detail?: Record<string, unknown>
}

export interface FomodPlugin {
  name?: string
  pluginName?: string
  displayName?: string
  file?: string
  selected?: boolean
  isSelected?: boolean
  confidence?: ConfidenceScore
  reasons?: FomodReason[]
}

export interface FomodGroup {
  name?: string
  plugins?: FomodPlugin[]
  deselected?: FomodPlugin[]
  confidence?: ConfidenceScore
  resolved_by?: string
  reasons?: FomodReason[]
}

export interface FomodStep {
  name?: string
  optionalFileGroups?: FomodGroup[]
  groups?: FomodGroup[]
  plugins?: FomodPlugin[]
  confidence?: ConfidenceScore
  visible?: boolean
  reasons?: FomodReason[]
}

export interface RunDiagnostics {
  confidence: ConfidenceScore
  exact_match: boolean
  phase_reached: string
  nodes_explored: number
  groups: { total: number; resolved_by_propagation: number; resolved_by_csp: number }
  repro: { missing: number; extra: number; size_mismatch: number; hash_mismatch: number; reproduced: number }
  timings_ms: { list: number; scan: number; solve: number; total: number }
  cache: { hit: boolean; source: string }
}

/**
 * @interface FomodFileEntry
 * @brief describe one normalized simulated output file.
 * @author Alex (https://github.com/lextpf)
 *
 * `path` is lowercase and slash-separated. `size` is in bytes.
 */
export interface FomodFileEntry {
  path: string
  size: number
  source?: string
}

export type FaultKind = 'missing' | 'hash_mismatch' | 'size_mismatch' | 'extra'

export const FAULT_RANK: Record<FaultKind, number> = {
  missing: 0,
  hash_mismatch: 1,
  size_mismatch: 2,
  extra: 3,
}

/**
 * @interface ReproDetail
 * @brief report bounded differences between simulated and installed output.
 * @author Alex (https://github.com/lextpf)
 *
 * `missing` paths have no output-tree entry. `truncated` and `total` appear
 * together after a cap.
 * truncated lists contain a stable path-sorted prefix, not a sample.
 */
export interface ReproDetail {
  missing: string[]
  extra: string[]
  size_mismatch: string[]
  hash_mismatch: string[]
  truncated?: boolean
  total?: number
}

export interface FomodDetail {
  moduleName?: string
  steps?: FomodStep[]
  diagnostics?: RunDiagnostics
  schema_version?: number
  updated?: number
  modified?: number
  outputTree?: FomodFileEntry[]
  outputTreeTruncated?: boolean
  outputTreeTotal?: number
  reproDetail?: ReproDetail
}
