export interface InstallationJob {
  id: string
  fileName: string
  status: 'pending' | 'uploading' | 'processing' | 'completed' | 'error'
  modPath?: string
  error?: string
  uploadProgress?: number
  processingStatus?: string
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

export interface FomodEntry {
  name: string
  size: number
  modified: number
  stepCount: number
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

export type ConfidenceBand = 'high' | 'medium' | 'low'

export interface ConfidenceComponents {
  evidence: number
  propagation: number
  repro: number
  ambiguity: number
}

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

export interface FomodDetail {
  moduleName?: string
  steps?: FomodStep[]
  diagnostics?: RunDiagnostics
  schema_version?: number
  updated?: number
  modified?: number
}
