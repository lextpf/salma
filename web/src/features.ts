// Nothing imports FEATURES, so neither flag does anything. The file stays as
// the record of why these two affordances are missing from the inspector: both
// need data the backend cannot produce.
//
// Turning one on is not a matter of flipping a flag. The MO2 plugin has to
// surface VFS or mod-state data through the API first, and Inspector then needs
// a new entry in its TabId union and its TAB_ITEMS list.
export const FEATURES = {
  conflicts: false, // G3: wins-over / lost-to vs other mods (needs MO2 VFS)
  stateFlags: false, // G4: Overwrites / Overwritten / Dirty / ... (needs MO2 mod state)
} as const
