/*!
 * @brief exposes the salma archive, inference, and installation engine.
 * @author Alex (https://github.com/lextpf)
 *
 * the crate exposes its supported boundary through the capi module.
 *
 * @verbatim
 * archive -> FOMOD IR -> atoms -> propagation -> CSP -> diagnostics
 * archive -> extraction -> FOMOD replay or content-root copy
 * @endverbatim
 */

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
