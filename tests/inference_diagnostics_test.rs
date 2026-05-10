//! Integration tests for the schema-v2 diagnostics wire format.
//!
//! Every case here drives the crate from outside, so it sees only what a
//! consumer of the emitted document sees. The structural asserts walk the owned
//! `Value` through its introspection accessors.
//!
//! What the nine cases pin:
//!
//! - `assemble_json_emits_schema_v2` - the document shape `assemble_json`
//!   produces: `schema_version`, the step/group/plugin skeleton, the per-plugin
//!   `selected` flag and reason chain, and the `diagnostics` object with its
//!   `timings_ms`, `repro` and `cache` children.
//! - `reason_code_names` - the `ReasonCode` to wire-name round trip.
//! - `confidence_band_thresholds` - the run band: high for a forced exact
//!   match, below high on the global-fallback path.
//! - `cache_hit_all_plugins_cache_reason` - the Tier-1 cache path, which puts a
//!   `FOMOD_PLUS_CACHE` reason on every plugin, selected and deselected alike.
//! - `propagator_records_forced_required` - the propagator records
//!   `FORCED_REQUIRED` for a Required plugin.
//! - `backward_compat_plugin_reader` - the schema-tolerant string-or-object
//!   plugin-entry reader.
//! - `repro_component_collapses_on_massive_misses`,
//!   `repro_component_size_mismatch_gets_half_credit` and
//!   `repro_component_flat_without_target_count` - the three repro-component
//!   cases.
//!
//! The confidence-formula micro-tests are not here. They need the private
//! helpers, so they live in the unit-test module of
//! `mo2_salma_rs::inference_diagnostics`: the boundary tables, the worked
//! group-composite example, the all-forced short-circuit, the run penalties,
//! the `reproduced` backfill branches and the `serialize_*` key-order tests.
//! Put a new formula test there and a new wire-shape test here.

use mo2_salma_rs::fomod_atom::{
    AtomIndex, ExpandedAtoms, FomodAtom, Origin, TargetFile, TargetTree,
};
use mo2_salma_rs::fomod_csp_types::{InferenceOverrides, SolverResult};
use mo2_salma_rs::fomod_inference_atoms::assemble_json;
use mo2_salma_rs::fomod_ir::FomodInstaller;
use mo2_salma_rs::fomod_ir_parser::{load_document, parse};
use mo2_salma_rs::fomod_propagator::{PropagationResult, propagate};
use mo2_salma_rs::inference_diagnostics::{
    InferenceDiagnosticsBuilder, ReasonCode, reason_code_to_string,
};
use mo2_salma_rs::json::Value;

use std::collections::HashSet;

/// Parse an inline `<config>` document into the IR. The empty archive prefix
/// leaves source paths exactly as the XML spells them, so a test can assert on
/// the literal strings it wrote.
fn parse_xml(xml: &str) -> FomodInstaller {
    let doc = load_document(xml).expect("load inline config xml");
    parse(&doc, "")
}

// ---------------------------------------------------------------------------
// Schema v2 wire format
// ---------------------------------------------------------------------------

#[test]
fn assemble_json_emits_schema_v2() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="G1" type="SelectAll">
              <plugins>
                <plugin name="P1">
                  <files><file source="a.dds" destination="textures/a.dds"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml);

    let result = SolverResult {
        selections: vec![vec![vec![true]]],
        exact_match: true,
        ..SolverResult::default()
    };

    let mut builder = InferenceDiagnosticsBuilder::new(&installer);
    builder.add_plugin_reason(0, 0, 0, ReasonCode::ForcedSelectAll, "SelectAll", None);
    builder.set_group_resolved_by(0, 0, "propagation.select_all");
    let dummy_prop = PropagationResult::default();
    builder.absorb_solver(&result);
    builder.finalize(&result, &dummy_prop, &installer);

    let j = assemble_json(&installer, &result, builder.diagnostics());

    assert_eq!(j.get("schema_version").and_then(Value::as_i64), Some(2));
    let steps = j.get("steps").expect("steps");
    assert!(steps.is_array());
    assert_eq!(steps.array_len(), Some(1));

    let step = steps.get_index(0).unwrap();
    assert_eq!(step.get("name").and_then(Value::as_str), Some("Step1"));
    assert!(step.contains("confidence"));
    assert!(
        step.get("confidence")
            .and_then(|c| c.get("band"))
            .unwrap()
            .is_string()
    );
    assert!(step.contains("visible"));

    let group = step
        .get("groups")
        .and_then(|g| g.get_index(0))
        .expect("group");
    assert_eq!(
        group.get("resolved_by").and_then(Value::as_str),
        Some("propagation.select_all")
    );
    assert!(group.get("plugins").unwrap().is_array());
    assert_eq!(group.get("plugins").unwrap().array_len(), Some(1));

    let plugin = group.get("plugins").and_then(|p| p.get_index(0)).unwrap();
    assert_eq!(plugin.get("name").and_then(Value::as_str), Some("P1"));
    assert_eq!(plugin.get("selected").and_then(Value::as_bool), Some(true));
    assert!(
        plugin
            .get("confidence")
            .and_then(|c| c.get("composite"))
            .unwrap()
            .is_number()
    );
    let reasons = plugin.get("reasons").unwrap();
    assert!(reasons.is_array());
    assert!(!reasons.is_empty());
    assert_eq!(
        reasons
            .get_index(0)
            .and_then(|r| r.get("code"))
            .and_then(Value::as_str),
        Some("FORCED_SELECT_ALL")
    );

    let diag = j.get("diagnostics").expect("diagnostics");
    assert!(diag.is_object());
    assert!(diag.get("timings_ms").unwrap().is_object());
    assert!(diag.get("repro").unwrap().is_object());
    assert!(diag.get("cache").unwrap().is_object());
    assert_eq!(
        diag.get("cache")
            .and_then(|c| c.get("hit"))
            .and_then(Value::as_bool),
        Some(false)
    );
}

// ---------------------------------------------------------------------------
// ReasonCode -> string round trip
// ---------------------------------------------------------------------------

#[test]
fn reason_code_names() {
    assert_eq!(
        reason_code_to_string(ReasonCode::ImplicitDefault),
        "IMPLICIT_DEFAULT"
    );
    assert_eq!(
        reason_code_to_string(ReasonCode::ForcedRequired),
        "FORCED_REQUIRED"
    );
    assert_eq!(
        reason_code_to_string(ReasonCode::UniqueFileEvidence),
        "UNIQUE_FILE_EVIDENCE"
    );
    assert_eq!(
        reason_code_to_string(ReasonCode::CspPhaseGreedy),
        "CSP_PHASE_GREEDY"
    );
    assert_eq!(
        reason_code_to_string(ReasonCode::FomodPlusCache),
        "FOMOD_PLUS_CACHE"
    );
}

// ---------------------------------------------------------------------------
// Confidence formula band thresholds
// ---------------------------------------------------------------------------

const BAND_XML: &str = r#"
    <config>
      <installSteps>
        <installStep name="S">
          <optionalFileGroups>
            <group name="G" type="SelectAll">
              <plugins>
                <plugin name="P">
                  <files><file source="a" destination="d"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

#[test]
fn confidence_band_thresholds() {
    let installer = parse_xml(BAND_XML);

    // Forced + exact match -> band high.
    {
        let result = SolverResult {
            selections: vec![vec![vec![true]]],
            exact_match: true,
            ..SolverResult::default()
        };
        let mut builder = InferenceDiagnosticsBuilder::new(&installer);
        builder.add_plugin_reason(0, 0, 0, ReasonCode::ForcedSelectAll, "SelectAll", None);
        let prop = PropagationResult::default();
        builder.absorb_solver(&result);
        builder.finalize(&result, &prop, &installer);
        assert_eq!(builder.diagnostics().run.confidence.band, "high");
    }

    // No reason + no exact match + csp.fallback path -> drops below high.
    {
        let result = SolverResult {
            selections: vec![vec![vec![true]]],
            exact_match: false,
            phase_reached: "csp.fallback".to_string(),
            phase_per_group: vec![vec!["csp.fallback".to_string()]],
            ..SolverResult::default()
        };
        let mut builder = InferenceDiagnosticsBuilder::new(&installer);
        let prop = PropagationResult::default();
        builder.absorb_solver(&result);
        builder.finalize(&result, &prop, &installer);
        assert_ne!(builder.diagnostics().run.confidence.band, "high");
    }
}

// ---------------------------------------------------------------------------
// Cache-hit path produces FOMOD_PLUS_CACHE on every plugin
// ---------------------------------------------------------------------------

#[test]
fn cache_hit_all_plugins_cache_reason() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Body">
          <optionalFileGroups>
            <group name="Skin" type="SelectExactlyOne">
              <plugins>
                <plugin name="A">
                  <files><file source="a" destination="da"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="B">
                  <files><file source="b" destination="db"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml);

    let result = SolverResult {
        selections: vec![vec![vec![true, false]]],
        ..SolverResult::default()
    };

    let mut builder = InferenceDiagnosticsBuilder::new(&installer);
    builder.set_cache_hit("fomod-plus");
    builder.absorb_solver(&result);
    let prop = PropagationResult::default();
    builder.finalize(&result, &prop, &installer);

    let j = assemble_json(&installer, &result, builder.diagnostics());

    let diag = j.get("diagnostics").unwrap();
    assert_eq!(
        diag.get("cache")
            .and_then(|c| c.get("hit"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        diag.get("cache")
            .and_then(|c| c.get("source"))
            .and_then(Value::as_str),
        Some("fomod-plus")
    );
    assert_eq!(
        diag.get("phase_reached").and_then(Value::as_str),
        Some("tier1_cache")
    );

    let group = j
        .get("steps")
        .and_then(|s| s.get_index(0))
        .and_then(|s| s.get("groups"))
        .and_then(|g| g.get_index(0))
        .unwrap();
    assert_eq!(group.get("plugins").unwrap().array_len(), Some(1));
    assert_eq!(group.get("deselected").unwrap().array_len(), Some(1));

    let has_cache_reason = |plugin: &Value| -> bool {
        plugin
            .get("reasons")
            .and_then(Value::array_len)
            .map(|n| {
                (0..n).any(|i| {
                    plugin
                        .get("reasons")
                        .and_then(|r| r.get_index(i))
                        .and_then(|r| r.get("code"))
                        .and_then(Value::as_str)
                        == Some("FOMOD_PLUS_CACHE")
                })
            })
            .unwrap_or(false)
    };
    assert!(has_cache_reason(
        group.get("plugins").and_then(|p| p.get_index(0)).unwrap()
    ));
    assert!(has_cache_reason(
        group
            .get("deselected")
            .and_then(|p| p.get_index(0))
            .unwrap()
    ));
}

// ---------------------------------------------------------------------------
// Propagator records FORCED_REQUIRED on Required plugins
// ---------------------------------------------------------------------------

#[test]
fn propagator_records_forced_required() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="S">
          <optionalFileGroups>
            <group name="G" type="SelectAny">
              <plugins>
                <plugin name="Req">
                  <files><file source="x" destination="dx"/></files>
                  <typeDescriptor><type name="Required"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml);

    let mut atoms = ExpandedAtoms::default();
    let mut doc_order = 0;
    for step in &installer.steps {
        for group in &step.groups {
            for plugin in &group.plugins {
                let mut plugin_atoms = Vec::new();
                for fe in &plugin.files {
                    let atom = FomodAtom {
                        source_path: fe.source.clone(),
                        dest_path: fe.destination.clone(),
                        priority: fe.priority,
                        document_order: doc_order,
                        file_size: 100,
                        origin: Origin::Plugin,
                        ..FomodAtom::default()
                    };
                    doc_order += 1;
                    plugin_atoms.push(atom);
                }
                atoms.per_plugin.push(plugin_atoms);
            }
        }
    }

    let mut atom_index: AtomIndex = AtomIndex::new();
    for plugin_atoms in &atoms.per_plugin {
        for a in plugin_atoms {
            atom_index
                .entry(a.dest_path.clone())
                .or_default()
                .push(a.clone());
        }
    }

    let mut target = TargetTree::new();
    target.insert("dx".to_string(), TargetFile { size: 100, hash: 0 });
    let excluded: HashSet<String> = HashSet::new();
    let overrides = InferenceOverrides::default();

    let prop = propagate(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        &overrides,
        None,
    );

    assert!(!prop.plugin_reasons.is_empty());
    assert_eq!(prop.plugin_reasons[0][0][0], ReasonCode::ForcedRequired);
}

// ---------------------------------------------------------------------------
// Backward-compat plugin reader (string and object forms)
// ---------------------------------------------------------------------------

/// Copy of `fomod_service::read_plugin_name`, which accepts either a bare
/// string or an object with a string `name` and yields an empty string for
/// anything else.
///
/// The original is private to its module, so this test carries its own copy.
/// Nothing keeps the two in step: change one and change the other.
fn read_plugin_name_local(entry: &Value) -> String {
    if entry.is_string() {
        return entry.as_str().unwrap().to_string();
    }
    if entry.is_object()
        && entry.contains("name")
        && entry.get("name").map(Value::is_string).unwrap_or(false)
    {
        return entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
    }
    String::new()
}

#[test]
fn backward_compat_plugin_reader() {
    let string_form = Value::string("Plugin1");
    assert_eq!(read_plugin_name_local(&string_form), "Plugin1");

    let mut object_form = Value::object();
    object_form.insert("name", Value::string("Plugin1"));
    object_form.insert("selected", Value::Bool(true));
    assert_eq!(read_plugin_name_local(&object_form), "Plugin1");

    let malformed = Value::Int(42);
    assert_eq!(read_plugin_name_local(&malformed), "");
}

// ---------------------------------------------------------------------------
// Repro component reflects tree-compare quality
// ---------------------------------------------------------------------------

const SINGLE_GROUP_XML: &str = r#"
<config>
  <installSteps>
    <installStep name="Step1">
      <optionalFileGroups>
        <group name="G1" type="SelectExactlyOne">
          <plugins>
            <plugin name="P1">
              <files><file source="a.wav" destination="sound/a.wav"/></files>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
</config>"#;

#[test]
fn repro_component_collapses_on_massive_misses() {
    let installer = parse_xml(SINGLE_GROUP_XML);

    let result = SolverResult {
        selections: vec![vec![vec![true]]],
        exact_match: false,
        missing: 35,
        ..SolverResult::default()
    };

    let mut builder = InferenceDiagnosticsBuilder::new(&installer);
    builder.absorb_solver(&result);
    builder.set_target_file_count(36);
    let dummy = PropagationResult::default();
    builder.finalize(&result, &dummy, &installer);

    let run = &builder.diagnostics().run;
    assert!(run.confidence.components.repro < 0.20);
    assert_eq!(run.repro.reproduced, 1);
    assert_eq!(run.confidence.band, "low");
}

#[test]
fn repro_component_size_mismatch_gets_half_credit() {
    let installer = parse_xml(SINGLE_GROUP_XML);

    let result = SolverResult {
        selections: vec![vec![vec![true]]],
        exact_match: false,
        size_mismatch: 5,
        ..SolverResult::default()
    };

    let mut builder = InferenceDiagnosticsBuilder::new(&installer);
    builder.absorb_solver(&result);
    builder.set_target_file_count(144);
    let dummy = PropagationResult::default();
    builder.finalize(&result, &dummy, &installer);

    let run = &builder.diagnostics().run;
    assert!(run.confidence.components.repro > 0.80);
    assert_eq!(run.repro.reproduced, 139);
}

#[test]
fn repro_component_flat_without_target_count() {
    let installer = parse_xml(SINGLE_GROUP_XML);

    let result = SolverResult {
        selections: vec![vec![vec![true]]],
        exact_match: false,
        missing: 35,
        ..SolverResult::default()
    };

    let mut builder = InferenceDiagnosticsBuilder::new(&installer);
    builder.absorb_solver(&result);
    let dummy = PropagationResult::default();
    builder.finalize(&result, &dummy, &installer);

    assert!((builder.diagnostics().run.confidence.components.repro - 0.85).abs() < 1e-9);
}
