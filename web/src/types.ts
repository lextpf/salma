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
