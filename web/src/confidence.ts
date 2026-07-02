import type { ConfidenceScore, FomodEntry, RunDiagnostics } from './types'

/**
 * @brief classify confidence for dashboard display.
 * @author Alex (https://github.com/lextpf)
 *
 * thresholds differ from serialized engine bands. missing data uses `LOW`
 * metadata with `hasData` false.
 */
export type Tier = 'EXACT' | 'HIGH' | 'PARTIAL' | 'LOW'

export interface TierInfo {
  tier: Tier
  label: Tier
  pct: number
  rank: 1 | 2 | 3 | 4
  grade: 'A' | 'B' | 'C' | 'D'
  bars: 1 | 3 | 4 | 5
  color: string
  hasData: boolean
  pill: { fg: string; bg: string; bd: string }
}

const TIER_META: Record<
  Tier,
  {
    rank: 1 | 2 | 3 | 4
    grade: 'A' | 'B' | 'C' | 'D'
    bars: 1 | 3 | 4 | 5
    color: string
    pill: { fg: string; bg: string; bd: string }
  }
> = {
  EXACT: {
    rank: 4,
    grade: 'A' as const,
    bars: 5,
    color: 'var(--tier-exact)',
    pill: { fg: 'var(--tier-exact)', bg: 'var(--tier-exact-bg)', bd: 'var(--tier-exact-bd)' },
  },
  HIGH: {
    rank: 3,
    grade: 'B' as const,
    bars: 4,
    color: 'var(--tier-high)',
    pill: { fg: 'var(--tier-high)', bg: 'var(--tier-high-bg)', bd: 'var(--tier-high-bd)' },
  },
  PARTIAL: {
    rank: 2,
    grade: 'C' as const,
    bars: 3,
    color: 'var(--tier-partial)',
    pill: {
      fg: 'var(--tier-partial)',
      bg: 'var(--tier-partial-bg)',
      bd: 'var(--tier-partial-bd)',
    },
  },
  LOW: {
    rank: 1,
    grade: 'D' as const,
    bars: 1,
    color: 'var(--tier-low)',
    pill: { fg: 'var(--tier-low)', bg: 'var(--tier-low-bg)', bd: 'var(--tier-low-bd)' },
  },
}

function compositeOf(c?: ConfidenceScore | number | null): number | null {
  if (c == null) {
    return null
  }
  if (typeof c === 'number') {
    return c
  }
  return typeof c.composite === 'number' ? c.composite : null
}

export function tierFor(input: {
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
}): TierInfo {
  const composite = compositeOf(input.confidence)
  const exact = input.exactMatch === true
  const hasData = exact || composite != null

  let tier: Tier
  if (exact) {
    tier = 'EXACT'
  } else if (composite != null && composite >= 0.85) {
    tier = 'HIGH'
  } else if (composite != null && composite >= 0.6) {
    tier = 'PARTIAL'
  } else {
    tier = 'LOW'
  }

  const value = composite ?? (exact ? 1 : 0)
  const meta = TIER_META[tier]
  return {
    tier,
    label: tier,
    pct: Math.round(value * 100),
    rank: meta.rank,
    grade: meta.grade,
    bars: meta.bars,
    color: meta.color,
    hasData,
    pill: meta.pill,
  }
}

export const tierFromEntry = (e: FomodEntry): TierInfo =>
  tierFor({ confidence: e.confidence, exactMatch: e.exactMatch })

export const tierFromDiagnostics = (d?: RunDiagnostics): TierInfo =>
  tierFor({ confidence: d?.confidence, exactMatch: d?.exact_match })
