/**
 * One archive the user dropped on the Install screen, tracked from selection to
 * a terminal state. Owned and mutated only by useInstallation.ts; nothing here
 * comes back from the server as a whole object.
 *
 * The state machine, and where each optional field arrives:
 *
 *   handleFileSelect
 *        |                       (id, fileName, createdAt, sizeBytes set here)
 *        v
 *    [ pending ] ---- cancel ----> [ error: 'Cancelled' ]
 *        |
 *   XHR upload starts
 *        v
 *   [ uploading ] -- xhr error / abort / non-2xx --> [ error ]
 *        |          (uploadProgress climbs 0 -> 100)
 *   upload 200 {modName}
 *        v
 *  [ processing ] -- poll every 1.5s, 200 times --> [ error: polling timed out ]
 *        |                    |
 *  !running && success  !running && !success
 *        v                    v
 *  [ completed ]          [ error ]
 *
 * Terminal states are `completed` and `error`; nothing leaves them. `error` is
 * also where cancellation lands, with the message 'Cancelled'.
 *
 * Jobs run one at a time, in selection order, because the server holds a single
 * install slot. Later jobs sit in `pending` until the one before them settles.
 */
export interface InstallationJob {
  id: string
  fileName: string
  status: 'pending' | 'uploading' | 'processing' | 'completed' | 'error'
  // Absolute destination path. Arrives only on completion, from the status poll.
  modPath?: string
  // The destination mod folder name. Returned by the upload response, so it is
  // known while the install is still running. modPath lands only on completion,
  // and the active card has to show where it is writing before then.
  modName?: string
  // Set only in the `error` state. 'Cancelled' when the user aborted.
  error?: string
  // Upload percentage in the range [0, 100]. Present from `uploading` onward.
  uploadProgress?: number
  // Human sentence for the active card. Not a status code; do not branch on it.
  processingStatus?: string
  // Locally stamped session metadata; the backend returns none of these. The
  // Install feed reads them for timestamps, duration and size. Both stamps are
  // milliseconds since the Unix epoch, from the browser clock. sizeBytes is the
  // selected file's size in bytes.
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
 * One row of GET /api/mo2/fomods: a cached inference record, not a mod.
 *
 * The server builds each row by scanning the FOMOD output directory for .json
 * files, so `size` and `modified` describe that JSON file and nothing else.
 * Do not present them as the mod's size or its install time.
 *
 * The server also sets `parseError: true` on a row whose JSON it could not
 * read. This interface deliberately does not declare that field, so the one
 * signal that `stepCount` and `confidence` are untrustworthy is invisible to
 * the UI. Add it here before writing anything that depends on those two.
 */
export interface FomodEntry {
  /** The JSON file's stem, and the id every /api/mo2/fomods/<name> call uses. */
  name: string
  /** Size of the cached JSON record in bytes. Not the mod's size. */
  size: number
  /** Last write time of the cached JSON record, in milliseconds since the Unix epoch. Not an install time. */
  modified: number
  /** Steps counted out of the cached record. 0 when the record failed to parse. */
  stepCount: number
  // Per-entry inference confidence in the range [0, 1], populated from the
  // cached FOMOD JSON's diagnostics. Absent for entries written before the
  // diagnostics schema, which is why every consumer must handle undefined.
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
 * The engine's own coarse label for a composite. Lowercase on the wire, and
 * banded at composite >= 0.85 ('high') and >= 0.5 ('medium').
 *
 * A different vocabulary from the UI's `Tier` in confidence.ts, which is
 * computed in the browser, cuts at 0.85 and 0.6, and adds `EXACT` from the
 * exact_match flag. Do not map one onto the other by name: a composite of 0.55
 * is 'medium' here and `LOW` there.
 */
export type ConfidenceBand = 'high' | 'medium' | 'low'

/**
 * The four confidence axes. Every field is in the range [0, 1]. None is a
 * percentage: multiply by 100 before displaying one.
 *
 * Two of the four are graded lookups rather than ratios, so an intermediate
 * value is a grade and not a proportion of anything countable. See
 * ConfidenceBreakdown.tsx for the wording users are shown, and
 * src/inference_diagnostics.rs for the grades themselves.
 */
export interface ConfidenceComponents {
  evidence: number
  propagation: number
  repro: number
  ambiguity: number
}

/**
 * A confidence readout for one plugin, group, step or run.
 *
 * `composite` is in the range [0, 1], not 0..100. It is a clamped weighted sum
 * of the four components, so averaging them evenly does not reconstruct it.
 * `band` is the engine's own label for that same number.
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

// One entry in a FOMOD's inferred virtual output tree, rendered by VfsTree.
//
// `path` is lowercased and `/`-separated by the inference pipeline's
// normalization, which is what makes it comparable with ReproDetail's buckets.
// `size` is in bytes. `source` is the path inside the archive the file was
// copied from; it is optional so a record written before the field existed still
// parses, and VfsTree falls back to the file name when it is missing.
export interface FomodFileEntry {
  path: string
  size: number
  source?: string
}

/** One way a file can diverge, worst first. Ordering drives the folder rollup. */
export type FaultKind = 'missing' | 'hash_mismatch' | 'size_mismatch' | 'extra'

/** Severity rank; lower is worse. Used to pick a folder's worst descendant. */
export const FAULT_RANK: Record<FaultKind, number> = {
  missing: 0,
  hash_mismatch: 1,
  size_mismatch: 2,
  extra: 3,
}

/**
 * Which destinations diverged from the installed mod, alongside the counts in
 * `diagnostics.repro`. The engine emits this unconditionally, with empty buckets
 * on a clean run, so empty means "reproduced everything".
 *
 * Paths are lowercased and `/`-separated, matching `FomodFileEntry.path`.
 * `missing` names files present in the installed mod that the inferred selection
 * does not produce, so those paths have no `outputTree` entry. The other three
 * buckets name paths that do.
 *
 * `truncated` and `total` appear together, and only when the engine capped the
 * lists. `total` is then the uncapped number of diverging destinations across
 * all four buckets, so the UI can say "and N more" rather than imply the lists
 * are complete. Truncation keeps a stable prefix of the path-sorted fault list,
 * not a sample.
 *
 * Watch the asymmetry with the output tree: these two markers sit inside
 * reproDetail, while outputTreeTruncated and outputTreeTotal are siblings of
 * outputTree.
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
  // Inferred install output tree, embedded by the backend during scan. Absent
  // for entries written before this field existed (Files tab degrades to empty).
  outputTree?: FomodFileEntry[]
  outputTreeTruncated?: boolean
  outputTreeTotal?: number
  reproDetail?: ReproDetail
}
