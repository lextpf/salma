//! `mo2_salma_rs` - the salma FOMOD engine: archive reading, FOMOD parsing,
//! selection inference, and install replay.
//!
//! The crate builds as a `cdylib` (`mo2_salma_rs.dll`, renamed to
//! `mo2-salma.dll` when it is packaged) and exposes exactly one boundary: the
//! flat C ABI in [`capi`]. Both consumers go through that boundary - the MO2
//! plugin `scripts/mo2-salma.py` through `ctypes`, and `mo2-server` through
//! `src/SalmaEngine.cpp`, which resolves the same symbols with `LoadLibrary`.
//! Improving the engine once therefore benefits both.
//!
//! ## Where to start
//!
//! The headline feature is inference: given a mod archive and the mod as it was
//! actually installed, recover which FOMOD options produced that layout.
//! [`fomod_inference_service`] orchestrates every stage of it and is the place to
//! start reading. If you are calling the engine rather than changing it, read
//! [`capi`] instead: it carries the ownership rules and the per-export failure
//! values, and nothing else is exported.
//!
//! [`logger`] writes `logs/salma.log` next to the DLL, or hands every message to
//! a host callback registered through [`capi::setLogCallback`].
//!
//! Many constructs in this crate look wrong and are load-bearing, because the
//! installed mod layouts the engine has to reproduce were produced by exactly
//! that behavior. Each one says so where it lives, and `PARITY-NOTES.md` is the
//! full record. Read the relevant section before changing engine behavior.
//!
//! ## Data flow
//!
//! The module list below is grouped by subject, not by reading order. This is
//! the order the data actually moves in:
//!
//! ```text
//!   inferFomodSelections  (capi)
//!         |
//!         v
//!   archive_service           list archive entries (zip / 7z / rar backends)
//!         |
//!         v
//!   fomod_ir_parser           fomod/ModuleConfig.xml -> fomod_ir
//!         |
//!         v
//!   fomod_inference_atoms     per-file atoms + atom index + installed-file
//!         |                   target tree
//!         v
//!   fomod_propagator          narrow each group's plugin domain
//!         |
//!         v
//!   fomod_csp_solver          five phases. Builds the read-only Precompute
//!         |                   (fomod_csp_precompute) from the propagation
//!         |                   result, calls fomod_csp_options to enumerate one
//!         |                   group, and fomod_forward_simulator to score every
//!         |                   candidate against the target tree
//!         v
//!   inference_diagnostics + json     schema-v2 JSON, or "" on any failure
//!
//!   install / installWithConfig  (capi)
//!         |
//!         v
//!   installation_service      extract to temp -> detect FOMOD -> cleanup
//!         |
//!         +--> fomod_service           replay -> file_operations
//!         +--> mod_structure_detector  non-FOMOD content-root copy
//!
//!   resolveModArchive  (capi)  ->  archive_resolver
//!
//!   Shared by several stages: fomod_dependency_evaluator (condition trees),
//!   fomod_atom, fomod_csp_types, fomod_ir, types, utils, logger.
//! ```
//!
//! ## Modules
//!
//! Foundations:
//!
//! - [`utils`] - path normalization, the path-safety guards, FNV-1a hashing,
//!   and the small string helpers every other module uses
//! - [`types`] - `FileOpType`, `FileOperation`, `InstallResult`, `PluginType`,
//!   `FomodDependencyContext`
//! - [`logger`] - the process-wide logger and the host callback
//! - [`json`] - the owned `Value` model and the serializer behind the schema-v2
//!   output. Its exact byte layout is part of that output's contract
//!
//! The FOMOD model:
//!
//! - [`fomod_ir`] - the IR structs: Installer -> Step -> Group -> Plugin ->
//!   FileEntry, with recursive condition trees
//! - [`fomod_ir_parser`] - the XML in `fomod/ModuleConfig.xml` to the IR,
//!   including the encoding autodetection applied when the document is loaded
//! - [`fomod_dependency_evaluator`] - evaluates condition trees and resolves a
//!   plugin's effective type; shared by the propagator, the solver and the
//!   simulator
//! - [`fomod_atom`] - the atom datatypes
//!
//! Inference:
//!
//! - [`fomod_inference_service`] - `infer_selections`, which drives every stage,
//!   plus the installed-file scan, the lazy contested-file hashing, the Tier-1
//!   `meta.ini` fomod-plus shortcut, and `compute_overrides`
//! - [`fomod_inference_atoms`] - atom expansion, the destination index, the
//!   target tree, and [`fomod_inference_atoms::assemble_json`]. The
//!   `serialize_*` helpers it composes live in [`inference_diagnostics`]
//! - [`fomod_propagator`] - the deterministic constraint-propagation pre-pass
//!   that narrows each group's plugin domain
//! - [`fomod_csp_types`] - the CSP solver datatypes: `ReproMetrics`,
//!   `InferenceOverrides`, `Precompute`, the option cache and the solver state,
//!   all consumed by [`fomod_csp_solver`]
//! - [`fomod_csp_precompute`] - `compute_evidence`, `build_precompute`, and the
//!   flag/condition helpers that build the solver's read-only view
//! - [`fomod_csp_options`] - per-group option enumeration and reduction, and the
//!   `SelectAny` caps
//! - [`fomod_csp_solver`] - the solve entry point: greedy, local search and
//!   repair, plus the iterative memoized backtracker, across five phases
//! - [`fomod_forward_simulator`] - replays a candidate selection in memory and
//!   diffs it against the target tree; the scoring oracle for every phase
//! - [`inference_diagnostics`] - `ReasonCode`, `ReasonDetail`, the confidence
//!   scoring, the `InferenceDiagnosticsBuilder` accumulator, and the schema-v2
//!   `serialize_*` helpers
//!
//! Archives and install:
//!
//! - [`archive_service`] - the archive I/O facade over the zip, 7z and rar
//!   backends. Entry order and path separators differ per backend and are
//!   load-bearing; the module doc explains each. `list_entries_with_sizes` has
//!   no in-repo oracle, so changes to it cannot be caught by the test suite
//!   alone; see `PARITY-NOTES.md`
//! - [`archive_resolver`] - the `installationFile` -> archive fallback chain
//!   behind `resolveModArchive`
//! - [`installation_service`] - the top-level orchestrator behind `install` and
//!   `installWithConfig`: extract to temp -> detect FOMOD -> replay or
//!   content-root copy -> cleanup
//! - [`fomod_service`] - the FOMOD install replay: dependency checks, the
//!   required, optional and conditional file passes, then priority-ordered
//!   execution
//! - [`mod_structure_detector`] - content-root detection for non-FOMOD archives
//! - [`file_operations`] - the queued copy/move executor behind every install

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
