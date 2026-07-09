import type { ConfidenceScore, FomodEntry, RunDiagnostics } from './types'

// Single source of truth for the v5 confidence tier shown across the Library
// (row meters, spec-rail dial + pill). EXACT is driven by the backend's
// authoritative exact_match flag (the install reproduced perfectly); the other
// tiers bucket the 0..1 composite score. Colors resolve to CSS vars defined per
// theme in index.css, so dark/light flip automatically.

export type Tier = 'EXACT' | 'HIGH' | 'PARTIAL' | 'LOW'

export interface TierInfo {
  tier: Tier
  label: Tier
  pct: number // rounded 0..100
  rank: 1 | 2 | 3 | 4 // number of filled SignalMeter bars
  color: string // CSS var for the tier's primary color (dial stroke, meter fill)
  hasData: boolean // false -> render a neutral meter and no pill (never a false LOW)
  pill: { fg: string; bg: string; bd: string }
}

const TIER_META: Record<
  Tier,
  { rank: 1 | 2 | 3 | 4; color: string; pill: { fg: string; bg: string; bd: string } }
> = {
  EXACT: {
    rank: 4,
    color: 'var(--tier-exact)',
    pill: { fg: 'var(--tier-exact-fg)', bg: 'var(--tier-exact-bg)', bd: 'var(--tier-exact-bd)' },
  },
  HIGH: {
    rank: 3,
    color: 'var(--tier-high)',
    pill: { fg: 'var(--tier-high-fg)', bg: 'var(--tier-high-bg)', bd: 'var(--tier-high-bd)' },
  },
  PARTIAL: {
    rank: 2,
    color: 'var(--tier-partial)',
    pill: {
      fg: 'var(--tier-partial-fg)',
      bg: 'var(--tier-partial-bg)',
      bd: 'var(--tier-partial-bd)',
    },
  },
  LOW: {
    rank: 1,
    color: 'var(--tier-low)',
    pill: { fg: 'var(--tier-low-fg)', bg: 'var(--tier-low-bg)', bd: 'var(--tier-low-bd)' },
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
    color: meta.color,
    hasData,
    pill: meta.pill,
  }
}

export const tierFromEntry = (e: FomodEntry): TierInfo =>
  tierFor({ confidence: e.confidence, exactMatch: e.exactMatch })

export const tierFromDiagnostics = (d?: RunDiagnostics): TierInfo =>
  tierFor({ confidence: d?.confidence, exactMatch: d?.exact_match })
