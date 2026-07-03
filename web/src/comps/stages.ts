/**
 * @brief map console operations and percentages onto install stages.
 * @author Alex (https://github.com/lextpf)
 *
 * keep stage order synchronized with `SessionFeed` icons and console ops.
 * `INSTALL`, `ERROR`, and unknown ops use `VALIDATE`.
 */
export const STAGES = ['VALIDATE', 'EXTRACT', 'SCAN', 'PARSE', 'INFER', 'WRITE'] as const

/**
 * @fn stageIndexForOp(op: string | null): number
 * @brief provide a safe index into the fixed stage sequence.
 * @author Alex (https://github.com/lextpf)
 *
 * values stay in [0, 5].
 */
export function stageIndexForOp(op: string | null): number {
  switch (op) {
    case 'EXTRACT':
      return 1
    case 'DETECT':
      return 2
    case 'PARSE':
      return 3
    case 'CACHE':
    case 'PROPAGATE':
    case 'CSP':
    case 'SIMULATE':
    case 'INFER':
      return 4
    case 'COPY':
    case 'WRITE':
    case 'DEPLOY':
    case 'CLEANUP':
    case 'DONE':
      return 5
    default:
      return 0
  }
}

export function segmentFills(
  stage: number,
  pct: number | null,
  tone: string,
  indeterminate: boolean,
): number[] {
  // percentages describe only the active operation.
  // failed and indeterminate active segments fill fully to keep state visible.
  return STAGES.map((_, i) => {
    if (tone === 'done') return 100
    if (i < stage) return 100
    if (i > stage) return 0
    if (tone === 'error') return 100
    if (indeterminate) return 100
    return pct ?? 0
  })
}
