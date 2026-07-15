//! `mo2_salma_rs` - Rust port of the salma `mo2-core` FOMOD engine.
//!
//! Milestone 1 ships only the flat C ABI skeleton: the eight `extern "C"`
//! exports declared in [`capi`], matching `src/CApi.hpp` symbol-for-symbol so
//! this DLL is a drop-in replacement for the C++ `mo2-salma.dll`. The engine
//! internals (inference, install replay, archive handling) arrive in later
//! tasks; for now every service-backed export returns a documented placeholder.

pub mod capi;
