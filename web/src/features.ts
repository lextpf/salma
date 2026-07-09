// Feature flags for v5 inspector affordances whose data the offline backend
// cannot produce yet. Flip these on once the MO2 plugin surfaces VFS / mod-state
// data through the API; the UI already renders the tabs as labelled placeholders.
export const FEATURES = {
  conflicts: false, // G3: wins-over / lost-to vs other mods (needs MO2 VFS)
  stateFlags: false, // G4: Overwrites / Overwritten / Dirty / ... (needs MO2 mod state)
} as const
