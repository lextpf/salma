//! `mo2_salma_rs` - Rust port of the salma `mo2-core` FOMOD engine.
//!
//! Milestone 1 ships the flat C ABI skeleton: the eight `extern "C"` exports
//! declared in [`capi`], matching `src/CApi.hpp` symbol-for-symbol so this DLL
//! is a drop-in replacement for the C++ `mo2-salma.dll`. The engine internals
//! (inference, install replay, archive handling) arrive task by task; every
//! service-backed export returns a documented placeholder until then.
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
//! - [`fomod_csp_types`] - `ReproMetrics` + `InferenceOverrides`, partial
//!   mirror of `src/FomodCSPTypes.hpp` and `src/FomodCSPSolver.hpp` (the rest
//!   of the CSP datatypes arrive in Tasks 8-9)
//! - [`fomod_forward_simulator`] - the forward install simulator and repro
//!   metrics, mirror of `src/FomodForwardSimulator.hpp`/`.cpp` plus the
//!   `compare_trees`/`collect_mismatched_dests` helpers from
//!   `src/FomodCSPSolver.cpp`
//! - [`fomod_propagator`] - the deterministic constraint-propagation pre-pass,
//!   mirror of `src/FomodPropagator.hpp`/`.cpp`
//! - [`inference_diagnostics`] - the `ReasonCode` enum + `ReasonDetail`, a
//!   partial mirror of `src/InferenceDiagnostics.hpp`/`.cpp` (the confidence
//!   scoring, accumulator, and schema-v2 JSON helpers arrive in Task 10)

pub mod capi;
pub mod fomod_atom;
pub mod fomod_csp_types;
pub mod fomod_dependency_evaluator;
pub mod fomod_forward_simulator;
pub mod fomod_inference_atoms;
pub mod fomod_ir;
pub mod fomod_ir_parser;
pub mod fomod_propagator;
pub mod inference_diagnostics;
pub mod types;
pub mod utils;
