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
//! - [`types`] - shared types from `src/Types.hpp` (currently `PluginType`)

pub mod capi;
pub mod types;
pub mod utils;
