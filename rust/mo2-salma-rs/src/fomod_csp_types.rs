//! CSP solver datatypes - partial Rust port of `src/FomodCSPTypes.hpp`.
//!
//! Task 6 ports only the two types the forward simulator and its scoring
//! oracle need:
//!
//! - [`ReproMetrics`] - reproduction quality counters (mirror of
//!   `mo2core::ReproMetrics` in `src/FomodCSPTypes.hpp`).
//! - [`InferenceOverrides`] - tri-state external-dependency overrides. Its C++
//!   home is `src/FomodCSPSolver.hpp` (NOT `FomodCSPTypes.hpp`); it is placed
//!   here so the simulator can consume it without pulling in the full solver
//!   header, which arrives in Tasks 8-9.
//!
//! The remainder of `FomodCSPTypes.hpp` (SolverState, Precompute, option
//! caches, memo keys, search plans, ...) lands with the CSP solver in Tasks
//! 8-9, which will EXTEND this module rather than replace it.

use crate::fomod_dependency_evaluator::ExternalConditionOverride;

/// Reproduction quality metrics comparing a simulated install to the target
/// file tree. Mirror of `mo2core::ReproMetrics` in `src/FomodCSPTypes.hpp`.
///
/// Counts are computed per destination file. Lower counts in every error
/// category means a better reproduction. Counters are `i32` to match the C++
/// `int` fields; all default to 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReproMetrics {
    /// Target files not produced by the simulation.
    pub missing: i32,
    /// Simulated files absent from the target.
    pub extra: i32,
    /// Files present in both but with differing (nonzero) sizes.
    pub size_mismatch: i32,
    /// Files matching in size but differing in (nonzero) content hash.
    pub hash_mismatch: i32,
    /// Files successfully reproduced (size and hash match, or fall through).
    pub reproduced: i32,
}

impl ReproMetrics {
    /// True when every error counter is zero (missing, extra, size_mismatch,
    /// hash_mismatch). Mirror of C++ `exact()`: `reproduced` is intentionally
    /// ignored, so an all-zero tree (nothing reproduced, nothing wrong) is
    /// still "exact".
    pub fn exact(&self) -> bool {
        self.missing == 0 && self.extra == 0 && self.size_mismatch == 0 && self.hash_mismatch == 0
    }

    /// Strict-weak (lexicographic) ordering, mirror of C++ `better_than`:
    /// `missing` asc, then `extra` asc, then `size_mismatch` asc, then
    /// `hash_mismatch` asc, then `reproduced` DESC (more reproduced is
    /// better). When all five compare equal, returns `false` (not strictly
    /// better than an equal metric).
    pub fn better_than(&self, rhs: &ReproMetrics) -> bool {
        if self.missing != rhs.missing {
            return self.missing < rhs.missing;
        }
        if self.extra != rhs.extra {
            return self.extra < rhs.extra;
        }
        if self.size_mismatch != rhs.size_mismatch {
            return self.size_mismatch < rhs.size_mismatch;
        }
        if self.hash_mismatch != rhs.hash_mismatch {
            return self.hash_mismatch < rhs.hash_mismatch;
        }
        if self.reproduced != rhs.reproduced {
            return self.reproduced > rhs.reproduced;
        }
        false
    }
}

/// Tri-state overrides for external dependencies the solver cannot evaluate.
/// Mirror of `mo2core::InferenceOverrides` (declared in
/// `src/FomodCSPSolver.hpp`).
///
/// Some FOMOD conditions depend on external state (game version, other
/// installed mods) unavailable during inference. These overrides let the
/// caller force individual conditional patterns or step-visibility conditions
/// to [`ExternalConditionOverride::ForceTrue`],
/// [`ExternalConditionOverride::ForceFalse`], or leave them
/// [`ExternalConditionOverride::Unknown`] so the solver can prune or explore
/// branches accordingly. Both vectors may be shorter than the installer's
/// counts; missing entries fall back to normal evaluation (see the forward
/// simulator).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InferenceOverrides {
    /// Per conditional pattern: tri-state external dependency override.
    /// Indexed by the pattern index in `FomodInstaller::conditional_patterns`.
    pub conditional_active: Vec<ExternalConditionOverride>,
    /// Per step visibility condition: tri-state external dependency override.
    /// Indexed by the step index in `FomodInstaller::steps`.
    pub step_visible: Vec<ExternalConditionOverride>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- defaults (must match the C++ in-struct initializers) ---

    #[test]
    fn repro_metrics_defaults_are_zero() {
        let m = ReproMetrics::default();
        assert_eq!(m.missing, 0);
        assert_eq!(m.extra, 0);
        assert_eq!(m.size_mismatch, 0);
        assert_eq!(m.hash_mismatch, 0);
        assert_eq!(m.reproduced, 0);
    }

    #[test]
    fn inference_overrides_default_is_empty() {
        let o = InferenceOverrides::default();
        assert!(o.conditional_active.is_empty());
        assert!(o.step_visible.is_empty());
    }

    // --- exact() ignores reproduced ---

    #[test]
    fn exact_true_when_all_errors_zero_regardless_of_reproduced() {
        assert!(ReproMetrics::default().exact());
        // reproduced > 0 with zero errors is still exact.
        assert!(
            ReproMetrics {
                reproduced: 999,
                ..Default::default()
            }
            .exact()
        );
        // zero reproduced and zero errors is also exact (empty vs empty).
        assert!(
            ReproMetrics {
                reproduced: 0,
                ..Default::default()
            }
            .exact()
        );
    }

    #[test]
    fn exact_false_when_any_error_nonzero() {
        for m in [
            ReproMetrics {
                missing: 1,
                ..Default::default()
            },
            ReproMetrics {
                extra: 1,
                ..Default::default()
            },
            ReproMetrics {
                size_mismatch: 1,
                ..Default::default()
            },
            ReproMetrics {
                hash_mismatch: 1,
                ..Default::default()
            },
        ] {
            assert!(!m.exact(), "{m:?} must not be exact");
        }
    }

    // --- better_than lexicographic table ---

    fn m(missing: i32, extra: i32, size: i32, hash: i32, repro: i32) -> ReproMetrics {
        ReproMetrics {
            missing,
            extra,
            size_mismatch: size,
            hash_mismatch: hash,
            reproduced: repro,
        }
    }

    #[test]
    fn better_than_orders_each_counter_in_priority() {
        // (a, b, a.better_than(b)) covering every tiebreak level.
        let cases: &[(ReproMetrics, ReproMetrics, bool)] = &[
            // missing dominates: fewer missing wins even with everything else worse.
            (m(1, 9, 9, 9, 0), m(2, 0, 0, 0, 999), true),
            (m(2, 0, 0, 0, 999), m(1, 9, 9, 9, 0), false),
            // extra breaks a missing tie.
            (m(3, 1, 9, 9, 0), m(3, 2, 0, 0, 999), true),
            (m(3, 2, 0, 0, 0), m(3, 1, 9, 9, 999), false),
            // size_mismatch breaks a missing+extra tie.
            (m(3, 3, 1, 9, 0), m(3, 3, 2, 0, 999), true),
            (m(3, 3, 2, 0, 0), m(3, 3, 1, 9, 999), false),
            // hash_mismatch breaks a missing+extra+size tie.
            (m(3, 3, 3, 1, 0), m(3, 3, 3, 2, 999), true),
            (m(3, 3, 3, 2, 0), m(3, 3, 3, 1, 999), false),
            // reproduced DESC is the final tiebreak.
            (m(3, 3, 3, 3, 5), m(3, 3, 3, 3, 4), true),
            (m(3, 3, 3, 3, 4), m(3, 3, 3, 3, 5), false),
            // fully equal -> false (irreflexive).
            (m(3, 3, 3, 3, 3), m(3, 3, 3, 3, 3), false),
            (ReproMetrics::default(), ReproMetrics::default(), false),
        ];
        for (a, b, expected) in cases {
            assert_eq!(a.better_than(b), *expected, "{a:?}.better_than({b:?})");
        }
    }

    #[test]
    fn better_than_is_irreflexive_for_arbitrary_values() {
        let samples = [
            ReproMetrics::default(),
            m(1, 2, 3, 4, 5),
            m(0, 0, 0, 0, 7),
            m(9, 0, 0, 0, 0),
        ];
        for s in samples {
            assert!(!s.better_than(&s), "{s:?} must not be better than itself");
        }
    }
}
