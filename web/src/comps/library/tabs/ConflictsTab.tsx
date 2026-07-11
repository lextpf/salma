import MIcon from '../../MIcon'
import { FEATURES } from '../../../features'

// Conflicts tab: wins-over / lost-to resolution against other mods. This needs
// the live MO2 virtual filesystem, which the offline server cannot observe, so
// the feature is flag-gated off and the tab renders a clear placeholder. When
// FEATURES.conflicts flips on, the real table is rendered here instead.
export default function ConflictsTab() {
  if (FEATURES.conflicts) {
    return null
  }

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        textAlign: 'center',
        gap: 12,
        padding: '48px 24px',
        border: '1px dashed var(--rule)',
        borderRadius: 9,
        background: 'var(--card)',
      }}
    >
      <div
        style={{
          width: 40,
          height: 40,
          borderRadius: '50%',
          border: '1px solid var(--rule-strong)',
          background: 'var(--paper-2)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--ink-4)',
        }}
      >
        <MIcon name="merge" size={20} />
      </div>
      <div style={{ fontSize: 'var(--fs-title)', fontWeight: 600, color: 'var(--ink-2)' }}>Conflicts</div>
      <p style={{ margin: 0, maxWidth: 360, fontSize: 'var(--fs-body)', lineHeight: 'var(--lh-body)', color: 'var(--ink-4)' }}>
        Conflict data requires MO2 plugin integration (coming soon).
      </p>
    </div>
  )
}
