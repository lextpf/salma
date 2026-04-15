//! `mo2_salma_rs` - Rust port of the salma `mo2-core` FOMOD engine.
//!
//! The crate exposes the flat C ABI in [`capi`]: the eight `extern "C"` exports
//! matching `src/CApi.hpp` symbol-for-symbol, so this DLL is a drop-in
//! replacement for the C++ `mo2-salma.dll`.
//!
//! All eight exports are now backed by real engine code. `inferFomodSelections`
//! drives the inference pipeline (Task 12) and `install` / `installWithConfig` /
//! `resolveModArchive` drive the install orchestrator and archive resolver
//! (Task 15). The one remaining gap is logging: there is no Rust `Logger` yet,
//! so the DLL emits nothing to `logs/salma.log` and never invokes a registered
//! `setLogCallback` (Task 17). Every dropped call site is marked in place with a
//! `// dropped log site` comment.
//!
//! Ported so far:
//! - [`utils`] - shared helpers, mirror of `src/Utils.hpp`/`src/Utils.cpp`
//! - [`types`] - shared types from `src/Types.hpp` (currently `PluginType`
//!   and `FomodDependencyContext`)
//! - [`fomod_ir`] - the FOMOD IR structs, mirror of `src/FomodIR.hpp`
//! - [`fomod_ir_parser`] - XML -> IR, mirror of `src/FomodIRParser.hpp`/`.cpp`
//!   plus the pugixml document-load (encoding autodetection) semantics
//! - [`fomod_dependency_evaluator`] - condition/plugin-type evaluation, mirror
//!   of `src/FomodDependencyEvaluator.hpp`/`.cpp`
//! - [`fomod_atom`] - atom datatypes, mirror of `src/FomodAtom.hpp`
//! - [`fomod_inference_atoms`] - atom expansion/indexing/target tree, mirror
//!   of `src/FomodInferenceAtoms.hpp`/`.cpp` (`assemble_json` lands with the
//!   diagnostics port in Task 10)
//! - [`fomod_csp_types`] - the CSP solver datatypes: `ReproMetrics` +
//!   `InferenceOverrides` (Task 6) plus the full `Precompute`/option-cache/
//!   solver-state type set (Task 8), mirror of `src/FomodCSPTypes.hpp`,
//!   `src/FomodCSPSolver.hpp`, and `src/FomodCSPSolverInternal.hpp` (the solve
//!   phases that consume these arrive in Task 9)
//! - [`fomod_csp_precompute`] - `compute_evidence` + `build_precompute` + the
//!   flag/condition helpers, mirror of `src/FomodCSPPrecompute.hpp`/`.cpp`
//! - [`fomod_csp_options`] - per-group option enumeration, reduction, and the
//!   SelectAny caps, mirror of `src/FomodCSPOptions.hpp`/`.cpp`
//! - [`fomod_csp_solver`] - the CSP solve entry point plus greedy/local-search/
//!   repair and the iterative memoized backtracker across the five phases,
//!   mirror of `src/FomodCSPSolver.cpp`/`src/FomodCSPSolverPhases.cpp`
//! - [`fomod_forward_simulator`] - the forward install simulator and repro
//!   metrics, mirror of `src/FomodForwardSimulator.hpp`/`.cpp` plus the
//!   `compare_trees`/`collect_mismatched_dests` helpers from
//!   `src/FomodCSPSolver.cpp`
//! - [`fomod_propagator`] - the deterministic constraint-propagation pre-pass,
//!   mirror of `src/FomodPropagator.hpp`/`.cpp`
//! - [`inference_diagnostics`] - the `ReasonCode` enum + `ReasonDetail` plus
//!   the confidence scoring, the `InferenceDiagnosticsBuilder` accumulator, and
//!   the schema-v2 `serialize_*` helpers, mirror of
//!   `src/InferenceDiagnostics.hpp`/`.cpp`
//! - [`json`] - the byte-faithful `nlohmann::json::dump(2)` replacement (owned
//!   `Value` model + serializer) used by the schema-v2 inference output path
//! - [`archive_service`] - the archive I/O facade (zip / 7z / rar backends),
//!   mirror of `src/ArchiveService.hpp`/`.cpp`, reproducing
//!   `list_entries_with_sizes` byte-for-byte against the golden corpus
//! - [`fomod_inference_service`] - the `infer_selections` orchestration that
//!   drives every stage, plus the installed-file scan, lazy contested-file
//!   hashing, the Tier-1 `meta.ini` fomod-plus shortcut, and `compute_overrides`,
//!   mirror of `src/FomodInferenceService.hpp`/`.cpp`
//! - [`file_operations`] - the queued copy/move executor behind every install,
//!   mirror of `src/FileOperations.hpp`/`.cpp`
//! - [`fomod_service`] - FOMOD install REPLAY (dependency checks, the required /
//!   optional / conditional file passes, and the priority-ordered execution),
//!   mirror of `src/FomodService.hpp`/`.cpp`
//! - [`mod_structure_detector`] - content-root detection for non-FOMOD
//!   archives, mirror of `src/ModStructureDetector.hpp`/`.cpp`
//! - [`archive_resolver`] - the `installationFile` -> archive fallback chain
//!   behind `resolveModArchive`, mirror of
//!   `src/FomodArchiveResolver.hpp`/`.cpp`
//! - [`installation_service`] - the top-level install orchestrator (extract to
//!   temp -> FOMOD detect -> replay or content-root copy -> cleanup) behind
//!   `install` / `installWithConfig`, mirror of
//!   `src/InstallationService.hpp`/`.cpp`

pub mod archive_resolver;
pub mod archive_service;
pub mod capi;
pub mod file_operations;
pub mod fomod_atom;
pub mod fomod_csp_options;
pub mod fomod_csp_precompute;
pub mod fomod_csp_solver;
pub mod fomod_csp_types;
pub mod fomod_dependency_evaluator;
pub mod fomod_forward_simulator;
pub mod fomod_inference_atoms;
pub mod fomod_inference_service;
pub mod fomod_ir;
pub mod fomod_ir_parser;
pub mod fomod_propagator;
pub mod fomod_service;
pub mod inference_diagnostics;
pub mod installation_service;
pub mod json;
pub mod logger;
pub mod mod_structure_detector;
pub mod types;
pub mod utils;
