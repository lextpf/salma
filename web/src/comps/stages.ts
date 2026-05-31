// The install pipeline's six stages, and how a console op and a percentage map
// onto them. Kept out of StageMeter.tsx so that file exports components only,
// which is what react-refresh needs to hot-reload it.
//
// Three vocabularies meet here and none of them is defined in this file:
//   - the input ops come from OP_RULES in useInstallConsole.ts,
//   - the output names are STAGES below,
//   - SessionFeed.tsx's STAGE_ICONS is indexed against those same six slots.
// Changing any one without the other two silently mislabels the meter, so the
// mapping is written out in full:
//
//   | index | STAGES   | ops that select it                                |
//   |-------|----------|---------------------------------------------------|
//   | 0     | VALIDATE | INSTALL, and anything the switch does not name    |
//   | 1     | EXTRACT  | EXTRACT                                           |
//   | 2     | SCAN     | DETECT                                            |
//   | 3     | PARSE    | PARSE                                             |
//   | 4     | INFER    | CACHE, PROPAGATE, CSP, SIMULATE, INFER            |
//   | 5     | WRITE    | COPY, WRITE, DEPLOY, CLEANUP, DONE                |
//
// Index 0 is the odd one: no op token is literally 'VALIDATE'. The stage name
// and the op vocabulary do not overlap there, so anything that looks like a
// "VALIDATE op" is really the default branch.
//
// 'ERROR' is not named either, and that has a visible consequence. OP_RULES is
// ordered, so a failure line that mentions a pipeline noun keeps that noun's op
// ("Extraction failed" -> EXTRACT -> stage 1) while a generic one falls through
// to ERROR and therefore to stage 0. A failed job's meter shows where it died
// only when the failure message happened to name the phase.

export const STAGES = ['VALIDATE', 'EXTRACT', 'SCAN', 'PARSE', 'INFER', 'WRITE'] as const

// Map a console op token to its stage index in the range [0, 5]. Unknown ops,
// the generic INSTALL fallback, ERROR, and the pre-extract upload phase all sit
// at 0. Never returns a value outside the range, so the caller can index STAGES
// with it directly.
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

/**
 * How full each of the six segments is, 0..100.
 *
 * Stages before the active one are complete, stages after it are untouched, and
 * the active one carries the current operation's percentage - which is the only
 * percentage the engine actually reports. A finished job fills every segment;
 * an indeterminate active stage fills its own segment so the hatch has
 * something to run across. Nothing here is interpolated to look busy.
 *
 * A failed job fills the segment named by `stage` rather than leaving it empty,
 * because the meter's job in that state is to show where the run stopped and an
 * empty segment reads as "not started yet". How closely `stage` locates the
 * failure depends on the op the failure line produced; see the note on 'ERROR'
 * above.
 *
 * `pct` is a percentage in the range [0, 100], or null when the engine has not
 * reported one; a null `pct` on the active stage fills it 0. `tone` comes from
 * computeInstallProgress and is 'done' or 'error' at the two terminal states
 * and 'normal' while running.
 */
export function segmentFills(
  stage: number,
  pct: number | null,
  tone: string,
  indeterminate: boolean,
): number[] {
  return STAGES.map((_, i) => {
    if (tone === 'done') return 100
    if (i < stage) return 100
    if (i > stage) return 0
    if (tone === 'error') return 100
    if (indeterminate) return 100
    return pct ?? 0
  })
}
