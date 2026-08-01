//! 1:1 port of the C++ GoogleTest suite `tests/fomod_inference_test.cpp`.
//!
//! Every `TEST(FomodInference, X)` case becomes a `#[test] fn x()` with the
//! behavior name in snake_case (`SelectAll_Deterministic` ->
//! `select_all_deterministic`). The C++ suite builds each fixture INLINE from an
//! XML string rather than reading anything off disk, and this port does the
//! same: no case touches `tests/golden/`, so the whole file runs on CI where no
//! corpus is present.
//!
//! The three C++ file-static helpers (`parse_xml`, `build_atoms`,
//! `build_target`) are ported below with identical semantics. `build_atoms` in
//! particular is NOT the production `expand_all_atoms`: it fabricates one atom
//! per file entry with a caller-chosen uniform size, walking required files,
//! then plugins in flat order, then conditional patterns, assigning
//! `document_order` from a single counter across all three passes. Several
//! assertions depend on that exact ordering.
//!
//! See PARITY-NOTES.md "Task 13" for the per-case mapping and for the cases
//! whose C++ form could not be expressed verbatim.

use std::collections::{HashMap, HashSet};

use mo2_salma_rs::fomod_atom::{ExpandedAtoms, FomodAtom, Origin, TargetFile, TargetTree};
use mo2_salma_rs::fomod_csp_solver::solve_fomod_csp;
use mo2_salma_rs::fomod_csp_types::InferenceOverrides;
use mo2_salma_rs::fomod_dependency_evaluator::{ExternalConditionOverride, evaluate_plugin_type};
use mo2_salma_rs::fomod_forward_simulator::{SimulatedTree, simulate};
use mo2_salma_rs::fomod_inference_atoms::{build_atom_index, expand_entry};
use mo2_salma_rs::fomod_inference_service::compute_overrides;
use mo2_salma_rs::fomod_ir::FomodInstaller;
use mo2_salma_rs::fomod_ir::{
    FomodCondition, FomodConditionType, FomodFileEntry, FomodGroup, FomodGroupType, FomodPlugin,
    FomodStep,
};
use mo2_salma_rs::fomod_ir_parser::parse_module_config;
use mo2_salma_rs::fomod_propagator::propagate;
use mo2_salma_rs::types::PluginType;

/// Parse an XML string into the installer IR. Mirror of the C++ `parse_xml`.
///
/// The C++ helper calls `doc.load_string(xml)` then `FomodIRParser::parse(doc,
/// prefix)`; the Rust parser takes bytes and does the decode internally, so the
/// equivalent is `parse_module_config(xml.as_bytes(), prefix)`. The C++ IGNORES
/// the `load_string` result, so a malformed document yields whatever partial
/// tree pugixml produced; no case in this suite relies on that, and every XML
/// here is well formed.
pub fn parse_xml(xml: &str, prefix: &str) -> FomodInstaller {
    parse_module_config(xml.as_bytes(), prefix).expect("test XML must parse")
}

/// Build synthetic `ExpandedAtoms` from the IR, one atom per file entry, all
/// with the same `file_size`. Mirror of the C++ `build_atoms`.
///
/// `document_order` runs from a SINGLE counter across the three passes in the
/// order required -> per-plugin (flat) -> per-conditional, which is what makes
/// conflict resolution deterministic in the cases that assert on it.
pub fn build_atoms(installer: &FomodInstaller, file_size: u64) -> ExpandedAtoms {
    let mut atoms = ExpandedAtoms::default();
    let mut doc_order: i32 = 0;

    for fe in &installer.required_files {
        atoms.required.push(FomodAtom {
            source_path: fe.source.clone(),
            dest_path: fe.destination.clone(),
            priority: fe.priority,
            document_order: doc_order,
            file_size,
            origin: Origin::Required,
            ..FomodAtom::default()
        });
        doc_order += 1;
    }

    let mut flat_idx: i32 = 0;
    for step in &installer.steps {
        for group in &step.groups {
            for plugin in &group.plugins {
                let mut plugin_atoms = Vec::new();
                for fe in &plugin.files {
                    plugin_atoms.push(FomodAtom {
                        source_path: fe.source.clone(),
                        dest_path: fe.destination.clone(),
                        priority: fe.priority,
                        document_order: doc_order,
                        file_size,
                        origin: Origin::Plugin,
                        plugin_index: flat_idx,
                        always_install: fe.always_install,
                        install_if_usable: fe.install_if_usable,
                        ..FomodAtom::default()
                    });
                    doc_order += 1;
                }
                atoms.per_plugin.push(plugin_atoms);
                flat_idx += 1;
            }
        }
    }

    for (ci, pattern) in installer.conditional_patterns.iter().enumerate() {
        let mut cond_atoms = Vec::new();
        for fe in &pattern.files {
            cond_atoms.push(FomodAtom {
                source_path: fe.source.clone(),
                dest_path: fe.destination.clone(),
                priority: fe.priority,
                document_order: doc_order,
                file_size,
                origin: Origin::Conditional,
                conditional_index: ci as i32,
                ..FomodAtom::default()
            });
            doc_order += 1;
        }
        atoms.per_conditional.push(cond_atoms);
    }

    atoms
}

/// Build a target tree with a uniform size and no hashes. Mirror of the C++
/// `build_target`.
pub fn build_target(paths: &[&str], file_size: u64) -> TargetTree {
    let mut tree = TargetTree::new();
    for p in paths {
        tree.insert(
            (*p).to_string(),
            TargetFile {
                size: file_size,
                hash: 0,
            },
        );
    }
    tree
}

/// The default uniform size both C++ helpers use.
pub const DEFAULT_SIZE: u64 = 100;

/// Convenience: the empty excluded-dest set most cases pass.
pub fn no_excluded() -> HashSet<String> {
    HashSet::new()
}

/// Count how many plugins are selected in a `[step][group][plugin]` grid.
pub fn count_selected(grid: &[Vec<Vec<bool>>]) -> usize {
    grid.iter()
        .flat_map(|s| s.iter())
        .flat_map(|g| g.iter())
        .filter(|p| **p)
        .count()
}

/// `dest -> winning source_path` for a simulated tree, for assertions that care
/// about which atom won a contested destination rather than the whole atom.
pub fn dest_to_source(tree: &SimulatedTree) -> HashMap<String, String> {
    tree.files
        .iter()
        .map(|(dest, atom)| (dest.clone(), atom.source_path.clone()))
        .collect()
}

/// Sorted destination list of a simulated tree.
pub fn sorted_dests(tree: &SimulatedTree) -> Vec<String> {
    let mut dests: Vec<String> = tree.files.keys().cloned().collect();
    dests.sort();
    dests
}

// ---------------------------------------------------------------------------
// Ported cases (C++ TEST order)
// ---------------------------------------------------------------------------

/// Test 1: Propagator resolves SelectAll without CSP.
///
/// Pins that a `SelectAll` group is fully resolved by the propagation pre-pass
/// alone (`fully_resolved`, one entry in `resolved_groups`) and that every
/// plugin survives in the narrowed domain.
#[test]
fn select_all_deterministic() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="Textures" type="SelectAll">
              <plugins>
                <plugin name="HiRes">
                  <files><file source="tex/hi.dds" destination="textures/hi.dds"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="LoRes">
                  <files><file source="tex/lo.dds" destination="textures/lo.dds"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="Normals">
                  <files><file source="tex/n.dds" destination="textures/n.dds"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);
    let atom_index = build_atom_index(&atoms);
    let target = build_target(
        &["textures/hi.dds", "textures/lo.dds", "textures/n.dds"],
        DEFAULT_SIZE,
    );
    let excluded = no_excluded();

    let overrides = InferenceOverrides {
        step_visible: vec![ExternalConditionOverride::Unknown; installer.steps.len()],
        conditional_active: vec![
            ExternalConditionOverride::Unknown;
            installer.conditional_patterns.len()
        ],
    };

    let result = propagate(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        &overrides,
        None,
    );

    assert!(result.fully_resolved);
    assert_eq!(result.resolved_groups.len(), 1);

    // All 3 plugins should be selected (SelectAll).
    let domain = &result.narrowed_domains[0][0];
    assert_eq!(domain.len(), 3);
    assert!(domain[0]);
    assert!(domain[1]);
    assert!(domain[2]);
}

/// Pins `evaluate_plugin_type` first-match-wins semantics: when two type
/// patterns carry the SAME condition, the earlier one in document order
/// decides the effective plugin type and the later one is never reached.
/// Port of `TEST(FomodInference, DependencyType_FirstPatternWins)`.
#[test]
fn dependency_type_first_pattern_wins() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="G1" type="SelectExactlyOne">
              <plugins>
                <plugin name="P1">
                  <files><file source="a.esp" destination="a.esp"/></files>
                  <typeDescriptor>
                    <dependencyType>
                      <defaultType name="Optional"/>
                      <patterns>
                        <pattern>
                          <dependencies>
                            <flagDependency flag="mode" value="advanced"/>
                          </dependencies>
                          <type name="Recommended"/>
                        </pattern>
                        <pattern>
                          <dependencies>
                            <flagDependency flag="mode" value="advanced"/>
                          </dependencies>
                          <type name="Required"/>
                        </pattern>
                      </patterns>
                    </dependencyType>
                  </typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    assert_eq!(installer.steps.len(), 1);
    let plugin = &installer.steps[0].groups[0].plugins[0];

    // Both patterns match (same condition), first should win.
    let flags: HashMap<String, String> =
        HashMap::from([("mode".to_string(), "advanced".to_string())]);
    let eff_type = evaluate_plugin_type(plugin, &flags, None);
    assert_eq!(eff_type, PluginType::Recommended);
}

/// `simulate()` installs the files of EVERY conditional pattern whose condition
/// holds, not just the first match: the single selected plugin sets both
/// `opt_a` and `opt_b`, so both patches land alongside the plugin's own file.
#[test]
fn conditional_all_matching_patterns_applied() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="G1" type="SelectExactlyOne">
              <plugins>
                <plugin name="P1">
                  <files><file source="base.esp" destination="base.esp"/></files>
                  <conditionFlags>
                    <flag name="opt_a">on</flag>
                    <flag name="opt_b">on</flag>
                  </conditionFlags>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
      <conditionalFileInstalls>
        <patterns>
          <pattern>
            <dependencies>
              <flagDependency flag="opt_a" value="on"/>
            </dependencies>
            <files><file source="patch_a.esp" destination="patch_a.esp"/></files>
          </pattern>
          <pattern>
            <dependencies>
              <flagDependency flag="opt_b" value="on"/>
            </dependencies>
            <files><file source="patch_b.esp" destination="patch_b.esp"/></files>
          </pattern>
        </patterns>
      </conditionalFileInstalls>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);

    // Select P1 (step 0, group 0, plugin 0).
    let selections = vec![vec![vec![true]]];

    let overrides = InferenceOverrides {
        conditional_active: vec![ExternalConditionOverride::ForceTrue; 2],
        step_visible: vec![ExternalConditionOverride::ForceTrue; 1],
    };

    let sim = simulate(&installer, &atoms, &selections, None, Some(&overrides));

    assert!(sim.files.contains_key("base.esp"));
    assert!(sim.files.contains_key("patch_a.esp"));
    assert!(sim.files.contains_key("patch_b.esp"));
}

/// Output-tree data contract. The Files tab is built by
/// `FomodInferenceService::add_output_tree`, which serializes `simulate()`'s
/// file map sorted by destination path. This guards that every selected plugin
/// file reaches the map and that the sort yields stable lexicographic order,
/// carrying the size + source each entry renders.
#[test]
fn output_tree_selected_files_sorted_by_dest() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="G1" type="SelectExactlyOne">
              <plugins>
                <plugin name="P1">
                  <files>
                    <file source="z/zebra.esp" destination="zebra.esp"/>
                    <file source="a/alpha.esp" destination="alpha.esp"/>
                    <file source="m/core.nif" destination="meshes/core.nif"/>
                  </files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);

    // Select P1 (step 0, group 0, plugin 0).
    let selections = vec![vec![vec![true]]];

    let overrides = InferenceOverrides {
        conditional_active: Vec::new(),
        step_visible: vec![ExternalConditionOverride::ForceTrue; 1],
    };

    let sim = simulate(&installer, &atoms, &selections, None, Some(&overrides));

    // Every selected file reaches the simulated tree (the output-tree source).
    assert_eq!(sim.files.len(), 3);
    assert!(sim.files.contains_key("alpha.esp"));
    assert!(sim.files.contains_key("zebra.esp"));
    assert!(sim.files.contains_key("meshes/core.nif"));

    // Replicate add_output_tree's sort: entries ordered by destination path.
    let dests = sorted_dests(&sim);

    assert_eq!(dests[0], "alpha.esp");
    assert_eq!(dests[1], "meshes/core.nif");
    assert_eq!(dests[2], "zebra.esp");

    // Each winning atom carries the size + source the Files tab renders.
    assert_eq!(sim.files["alpha.esp"].file_size, 100);
    assert_eq!(sim.files["alpha.esp"].source_path, "a/alpha.esp");
    assert_eq!(sim.files["zebra.esp"].source_path, "z/zebra.esp");
    assert_eq!(sim.files["meshes/core.nif"].source_path, "m/core.nif");
}

/// Test 4: Parser orders steps alphabetically when no order attribute.
///
/// Pins that `<installSteps>` with no `order` attribute defaults to Ascending,
/// so the three document-order steps Zeta / Alpha / Mid come back out of the IR
/// sorted by name.
#[test]
fn order_default_ascending() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Zeta">
          <optionalFileGroups>
            <group name="G" type="SelectAny">
              <plugins>
                <plugin name="P">
                  <files><file source="z.esp" destination="z.esp"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Alpha">
          <optionalFileGroups>
            <group name="G" type="SelectAny">
              <plugins>
                <plugin name="P">
                  <files><file source="a.esp" destination="a.esp"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Mid">
          <optionalFileGroups>
            <group name="G" type="SelectAny">
              <plugins>
                <plugin name="P">
                  <files><file source="m.esp" destination="m.esp"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    assert_eq!(installer.steps.len(), 3);
    assert_eq!(installer.steps[0].name, "Alpha");
    assert_eq!(installer.steps[1].name, "Mid");
    assert_eq!(installer.steps[2].name, "Zeta");
}

// ---------------------------------------------------------------------------
// Test 5: Domain widening skipped when step override is ForceTrue
// ---------------------------------------------------------------------------

/// Pins that an explicit `ForceTrue` step-visibility override does not cause the
/// solver to widen a domain the propagator already narrowed: a
/// `SelectExactlyOne` group with one `Optional` and two `NotUsable` plugins is
/// resolved to the single usable plugin by propagation alone, and the CSP solve
/// seeded with that propagation result keeps exactly that selection.
#[test]
fn domain_widening_respects_evidence() {
    // Build a group with 3 plugins where type evaluation makes only 1 usable.
    // With ForceTrue step override, widening should NOT happen - domain stays narrow.
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="G1" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="Usable">
                  <files><file source="u.esp" destination="u.esp"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="NotUsable1">
                  <files><file source="n1.esp" destination="n1.esp"/></files>
                  <typeDescriptor><type name="NotUsable"/></typeDescriptor>
                </plugin>
                <plugin name="NotUsable2">
                  <files><file source="n2.esp" destination="n2.esp"/></files>
                  <typeDescriptor><type name="NotUsable"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);
    let atom_index = build_atom_index(&atoms);
    let target = build_target(&["u.esp"], DEFAULT_SIZE);
    let excluded = no_excluded();

    // With ForceTrue override, propagation should resolve the single usable plugin.
    let overrides = InferenceOverrides {
        step_visible: vec![ExternalConditionOverride::ForceTrue],
        conditional_active: vec![],
    };

    let result = propagate(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        &overrides,
        None,
    );
    assert!(result.fully_resolved);

    let domain = &result.narrowed_domains[0][0];
    assert!(domain[0]); // Usable
    assert!(!domain[1]); // NotUsable1
    assert!(!domain[2]); // NotUsable2

    // CSP solver should also respect this: with ForceTrue, it should not widen.
    let solver_result = solve_fomod_csp(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        Some(&overrides),
        Some(&result),
    );
    // Only the usable plugin should be selected.
    assert_eq!(solver_result.selections.len(), 1);
    assert_eq!(solver_result.selections[0].len(), 1);
    assert_eq!(solver_result.selections[0][0].len(), 3);
    assert!(solver_result.selections[0][0][0]);
    assert!(!solver_result.selections[0][0][1]);
    assert!(!solver_result.selections[0][0][2]);
}

/// Test 6: Propagator eliminates plugins with no target evidence.
///
/// Pins rule 2 (file evidence) feeding rule 3 (cardinality): in a
/// `SelectExactlyOne` group where every plugin owns a group-unique dest, the
/// plugins whose unique dests are absent from the target tree are eliminated,
/// leaving a single usable plugin, which resolves the group and therefore the
/// whole installer without any CSP solve.
#[test]
fn propagator_narrows_domains() {
    // SelectExactlyOne group with 3 plugins, each with unique files.
    // Only plugin B's files are in the target.
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="Textures" type="SelectExactlyOne">
              <plugins>
                <plugin name="PluginA">
                  <files><file source="a/tex.dds" destination="textures/a.dds"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="PluginB">
                  <files><file source="b/tex.dds" destination="textures/b.dds"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="PluginC">
                  <files><file source="c/tex.dds" destination="textures/c.dds"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);
    let atom_index = build_atom_index(&atoms);
    // Only plugin B's file is in the target.
    let target = build_target(&["textures/b.dds"], DEFAULT_SIZE);
    let excluded = no_excluded();

    let overrides = InferenceOverrides {
        step_visible: vec![ExternalConditionOverride::Unknown],
        conditional_active: Vec::new(),
    };

    let result = propagate(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        &overrides,
        None,
    );

    // A and C should be eliminated (unique atoms miss target), B survives.
    // SelectExactlyOne with 1 usable => resolved.
    assert!(result.fully_resolved);
    let domain = &result.narrowed_domains[0][0];
    assert!(!domain[0]); // A eliminated
    assert!(domain[1]); // B survives
    assert!(!domain[2]); // C eliminated
}

/// Pins that the propagator reaches a fixpoint across two steps: step 1's
/// SelectAll group resolves and publishes its `conditionFlags`, and step 2's
/// SelectExactlyOne group ends up resolved to the single surviving plugin.
///
/// Ported 1:1 from `TEST(FomodInference, PropagatorFlagPropagation)`.
#[test]
fn propagator_flag_propagation() {
    // Step 1: SelectAll group with 1 plugin that sets flag "mode"="advanced".
    // Step 2: SelectExactlyOne group with 2 plugins:
    //   - Plugin A: dependencyType pattern checks flag mode=advanced => NotUsable
    //   - Plugin B: dependencyType pattern checks flag mode=advanced => Required
    // After resolving step 1, propagator should propagate flags and resolve step 2.
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="Setup" type="SelectAll">
              <plugins>
                <plugin name="Init">
                  <files><file source="init.esp" destination="init.esp"/></files>
                  <conditionFlags>
                    <flag name="mode">advanced</flag>
                  </conditionFlags>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Step2">
          <optionalFileGroups>
            <group name="Options" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="BasicPatch">
                  <files><file source="basic.esp" destination="basic.esp"/></files>
                  <typeDescriptor>
                    <dependencyType>
                      <defaultType name="Optional"/>
                      <patterns>
                        <pattern>
                          <dependencies>
                            <flagDependency flag="mode" value="advanced"/>
                          </dependencies>
                          <type name="NotUsable"/>
                        </pattern>
                      </patterns>
                    </dependencyType>
                  </typeDescriptor>
                </plugin>
                <plugin name="AdvancedPatch">
                  <files><file source="advanced.esp" destination="advanced.esp"/></files>
                  <typeDescriptor>
                    <dependencyType>
                      <defaultType name="Optional"/>
                      <patterns>
                        <pattern>
                          <dependencies>
                            <flagDependency flag="mode" value="advanced"/>
                          </dependencies>
                          <type name="Required"/>
                        </pattern>
                      </patterns>
                    </dependencyType>
                  </typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);
    let atom_index = build_atom_index(&atoms);
    let target = build_target(&["init.esp", "advanced.esp"], DEFAULT_SIZE);
    let excluded = no_excluded();

    let overrides = InferenceOverrides {
        step_visible: vec![ExternalConditionOverride::Unknown; installer.steps.len()],
        conditional_active: Vec::new(),
    };

    let result = propagate(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        &overrides,
        None,
    );

    // Step 1: SelectAll resolves Init (iteration 1).
    // Flag mode=advanced propagates.
    // Step 2: BasicPatch becomes NotUsable, AdvancedPatch is only option => resolved (iteration 2).
    assert!(result.fully_resolved);
    assert_eq!(result.resolved_groups.len(), 2);

    // Step 1, group 0: Init selected
    assert!(result.narrowed_domains[0][0][0]);

    // Step 2, group 0: BasicPatch eliminated, AdvancedPatch selected
    assert!(!result.narrowed_domains[1][0][0]);
    assert!(result.narrowed_domains[1][0][1]);
}

/// Pins that the simulator's `install_if_usable` handling matches the real
/// installer's chronological flag order: a SELECTED plugin's `installIfUsable`
/// atom is queued at its own step's enqueue time, so a flag set by a LATER
/// plugin cannot retroactively make it NotUsable and suppress it. Regression
/// for the CBBE 3BA exact=true vs real-test-fail divergence.
///
/// Pre-Patch-A bug: simulate_into ran a global Phase 3 that evaluated each
/// plugin's eff_type with flags accumulated through Phase 2 from EVERY selected
/// plugin (including ones lexically after the plugin being evaluated). When P1
/// has an install_if_usable atom whose dependencyType resolves to NotUsable
/// only after a flag set by P2 (later), the simulator suppressed P1's atom.
/// The real installer's enqueue_plugin_files queues P1's install_if_usable
/// atom unconditionally for a SELECTED plugin.
#[test]
fn simulate_flag_order_for_install_if_usable_matches_real_installer() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="G1" type="SelectExactlyOne">
              <plugins>
                <plugin name="P1">
                  <files>
                    <file source="x.dat" destination="x.dat" installIfUsable="true"/>
                  </files>
                  <typeDescriptor>
                    <dependencyType>
                      <defaultType name="Optional"/>
                      <patterns>
                        <pattern>
                          <dependencies>
                            <flagDependency flag="F" value="On"/>
                          </dependencies>
                          <type name="NotUsable"/>
                        </pattern>
                      </patterns>
                    </dependencyType>
                  </typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Step2">
          <optionalFileGroups>
            <group name="G2" type="SelectExactlyOne">
              <plugins>
                <plugin name="P2">
                  <files>
                    <file source="y.dat" destination="y.dat"/>
                  </files>
                  <conditionFlags>
                    <flag name="F">On</flag>
                  </conditionFlags>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);

    // Both P1 and P2 selected.
    let selections = vec![vec![vec![true]], vec![vec![true]]];

    let overrides = InferenceOverrides {
        step_visible: vec![ExternalConditionOverride::ForceTrue; installer.steps.len()],
        conditional_active: vec![
            ExternalConditionOverride::Unknown;
            installer.conditional_patterns.len()
        ],
    };

    let sim = simulate(&installer, &atoms, &selections, None, Some(&overrides));

    // P1's install_if_usable atom must be present even though P2 sets F=On.
    // The real installer queues P1's entry at step-1 enqueue time when F is
    // still unset (eff_type Optional), so x.dat ends up installed.
    assert!(
        sim.files.contains_key("x.dat"),
        "install_if_usable atom from P1 was suppressed by flag set by a later plugin (P2)"
    );
    assert!(sim.files.contains_key("y.dat"));
}

/// `compute_overrides` leaves duplicate-name conditional sibling steps as
/// `Unknown` (regression for the Schlongs of Skyrim "all 7 steps ForceTrue"
/// bug).
///
/// Two installSteps both named "Skin Texture", each with a single plugin whose
/// folder source produces the same dest paths. With the old "any atom hits
/// target -> ForceTrue" rule, both steps got ForceTrue and the simulator ran
/// them both, with last-applied content winning. The new rule requires at
/// least one dest unique to the step.
#[test]
fn overrides_duplicate_name_steps_leave_unknown() {
    let mut installer = FomodInstaller::default();

    // Two siblings with identical dest sets.
    for i in 0..2 {
        let mut step = FomodStep {
            name: "Skin Texture".to_string(),
            visible: Some(FomodCondition {
                r#type: FomodConditionType::Flag,
                flag_name: "BodyBuilder".to_string(),
                flag_value: if i == 0 { "Off" } else { "On" }.to_string(),
                ..FomodCondition::default()
            }),
            ..FomodStep::default()
        };

        let group = FomodGroup {
            name: "Skin Texture".to_string(),
            r#type: FomodGroupType::SelectExactlyOne,
            ..FomodGroup::default()
        };

        let plugin = FomodPlugin {
            name: "Hairless".to_string(),
            r#type: PluginType::Optional,
            ..FomodPlugin::default()
        };

        step.groups.push(group);
        step.groups[0].plugins.push(plugin);
        installer.steps.push(step);
    }

    // Two atoms per step, all sharing the same dest set.
    let mut atoms = ExpandedAtoms::default();
    atoms.per_plugin.resize(2, Vec::new());
    let make_atom = |src: &str, dst: &str, doc: i32| FomodAtom {
        source_path: src.to_string(),
        dest_path: dst.to_string(),
        document_order: doc,
        file_size: DEFAULT_SIZE,
        origin: Origin::Plugin,
        ..FomodAtom::default()
    };
    atoms.per_plugin[0].push(make_atom("off/textures/body.dds", "textures/body.dds", 0));
    atoms.per_plugin[0].push(make_atom("off/textures/face.dds", "textures/face.dds", 1));
    atoms.per_plugin[1].push(make_atom("on/textures/body.dds", "textures/body.dds", 2));
    atoms.per_plugin[1].push(make_atom("on/textures/face.dds", "textures/face.dds", 3));

    let idx = build_atom_index(&atoms);
    let target = build_target(&["textures/body.dds", "textures/face.dds"], DEFAULT_SIZE);
    let excluded = no_excluded();

    let overrides = compute_overrides(&installer, &atoms, &idx, &target, &excluded);

    assert_eq!(overrides.step_visible.len(), 2);
    assert_eq!(
        overrides.step_visible[0],
        ExternalConditionOverride::Unknown
    );
    assert_eq!(
        overrides.step_visible[1],
        ExternalConditionOverride::Unknown
    );
}

/// `compute_overrides` forces step visibility True when the step has at least
/// one dest unique to it (regression guard for the simple case).
///
/// The fixture is built directly against the IR rather than from XML: a single
/// step with one SelectExactlyOne group holding one Optional plugin, whose only
/// atom lands on a dest that is present in the target tree and reached by no
/// other step. That makes `dest_step_count["file.dat"] == 1`, the sole trigger
/// for `ForceTrue` on `step_visible[0]`.
#[test]
fn overrides_unique_dest_forces_true() {
    let mut installer = FomodInstaller::default();
    let mut step = FomodStep {
        name: "Step1".to_string(),
        ..FomodStep::default()
    };

    let group = FomodGroup {
        name: "G1".to_string(),
        r#type: FomodGroupType::SelectExactlyOne,
        ..FomodGroup::default()
    };

    let plugin = FomodPlugin {
        name: "P1".to_string(),
        r#type: PluginType::Optional,
        ..FomodPlugin::default()
    };

    step.groups.push(group);
    step.groups[0].plugins.push(plugin);
    installer.steps.push(step);

    let mut atoms = ExpandedAtoms::default();
    atoms.per_plugin.resize(1, Vec::new());
    let a = FomodAtom {
        source_path: "src/file.dat".to_string(),
        dest_path: "file.dat".to_string(),
        document_order: 0,
        file_size: 100,
        origin: Origin::Plugin,
        ..FomodAtom::default()
    };
    atoms.per_plugin[0].push(a);

    let idx = build_atom_index(&atoms);
    let target = build_target(&["file.dat"], DEFAULT_SIZE);
    let excluded = no_excluded();

    let overrides = compute_overrides(&installer, &atoms, &idx, &target, &excluded);

    assert_eq!(overrides.step_visible.len(), 1);
    assert_eq!(
        overrides.step_visible[0],
        ExternalConditionOverride::ForceTrue
    );
}

/// Regression: branch-and-bound lower bound must treat a conditional-only dest
/// as still fixable while a flag-setter group remains unassigned (finding 2.2,
/// conditional-only dest admissibility).
///
/// The lower bound's dest_to_groups / dest_to_size_match_groups /
/// dest_to_hash_capable_groups maps are populated from Plugin-origin atoms only.
/// A dest that some path produces solely via a conditionalFileInstalls pattern
/// (gated on a flag set by a LATER group) has no entry there, so the buggy bound
/// counts it as an unfixable miss and cannot_beat() prunes the subtree that
/// would have reached the exact solution.
///
/// This test must exercise the backtracking DFS, where lower_bound() runs. The
/// scenario is therefore built as a genuine local optimum that greedy + local
/// search CANNOT escape with a single group flip, so the solver is forced past
/// Phase-1 into the global backtrack pass where the bound fires.
///
/// Scenario (6 SelectExactlyOne groups, steps Step0..Step5):
///   - Step0: "Decoy0" directly produces the conditional dest "cond/special.dat"
///     AND an unwanted "decoy/extra.dat"; "Clean0" produces only "base/0.dat".
///     Decoy0 out-scores Clean0 (it uniquely supplies cond/special.dat), so
///     greedy picks Decoy0 -> missing=0, extra=1 (the decoy file).
///   - Step5 (LAST group, highest order position): "Plain5" directly produces
///     "base/5.dat" and "Flag5" produces no files but sets flag cond=on. The
///     conditionalFileInstalls pattern gated on cond=on produces BOTH
///     "base/5.dat" and "cond/special.dat". Because Plain5 is the unique plugin
///     producer of base/5.dat (evidence 3) it out-scores Flag5 (evidence 2 from
///     the conditional flag link), so greedy leaves the flag OFF and picks
///     Plain5. cond/special.dat is then supplied only by Decoy0.
///
/// The greedy result {Decoy0, Clean1..4, Plain5} is a STRICT local optimum at
/// {missing:0, extra:1} that no single flip improves:
///   - Flip Step0 Decoy0 -> Clean0: drops decoy/extra.dat but also drops
///     cond/special.dat (conditional still OFF because Plain5 is selected) ->
///     {missing:1, extra:0}, not better.
///   - Flip Step5 Plain5 -> Flag5: fires the conditional (cond/special.dat now
///     redundant, base/5.dat now from the pattern) but leaves decoy/extra.dat ->
///     {missing:0, extra:1}, not better.
///
/// The exact reproduction needs BOTH changes at once (Clean0 AND Flag5), a
/// 2-coordinated move that hill-climbing cannot reach. targeted_repair only
/// touches SelectAny/AtLeastOne groups, so Phase-1 gives up here and the solver
/// escalates to the global backtracking DFS.
///
/// Why the buggy bound prunes the exact path: the bound fires at order position 4
/// (group G4). On the prefix that chose Clean0 (not Decoy0) with Step5 still at
/// its best value Plain5, "cond/special.dat" is missing from the partial sim.
/// Its only dest_to_groups entry is Step0's Decoy0, already past in the order, so
/// has_remaining_group() is false; the conditional's flag setter (Step5, position
/// 5) is invisible to dest_to_groups. The buggy can_fix_missing reports it
/// unfixable -> lb.missing = 1 > best.missing = 0 -> cannot_beat() prunes before
/// Step5=Flag5 is ever tried, so exact_match stays false on revert.
///
/// The fix's conditional_repair_remaining() sees that "cond/special.dat" is a
/// conditional dest, that flag "cond" is needed, and that its setter group
/// (Step5) is still unassigned (order_pos >= next_idx), so it treats the dest as
/// fixable -> lb.missing = 0 -> the subtree survives and the exact is found.
///
/// Step3 carries a second decoy "Swap3" that produces cond/special.dat instead
/// of base/3.dat (it survives propagation because it only ever supplies a target
/// file). Greedy prefers Clean3 (the unique producer of base/3.dat), but the DFS
/// still explores the Swap3 branch inside the Decoy0 subtree. On that branch
/// base/3.dat is missing and its only producer group (Step3) is already past in
/// the order, so the bound prunes it on genuine, non-conditional grounds. This
/// guarantees lower_bound > 0 in the pruning summary even WITH the fix present,
/// proving the DFS bound path actually executed.
#[test]
fn conditional_dest_flag_set_by_later_group_reaches_exact() {
    let xml = r#"
    <config>
      <installSteps>
        <installStep name="Step0">
          <optionalFileGroups>
            <group name="G0" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="Decoy0">
                  <files>
                    <file source="d0/base.dat" destination="base/0.dat"/>
                    <file source="d0/extra.dat" destination="decoy/extra.dat"/>
                    <file source="d0/special.dat" destination="cond/special.dat"/>
                  </files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="Clean0">
                  <files><file source="c0/base.dat" destination="base/0.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Step1">
          <optionalFileGroups>
            <group name="G1" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="Clean1">
                  <files><file source="c1/base.dat" destination="base/1.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="Alt1">
                  <files><file source="a1/alt.dat" destination="alt/1.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Step2">
          <optionalFileGroups>
            <group name="G2" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="Clean2">
                  <files><file source="c2/base.dat" destination="base/2.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="Alt2">
                  <files><file source="a2/alt.dat" destination="alt/2.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Step3">
          <optionalFileGroups>
            <group name="G3" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="Clean3">
                  <files><file source="c3/base.dat" destination="base/3.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="Swap3">
                  <files><file source="s3/special.dat" destination="cond/special.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Step4">
          <optionalFileGroups>
            <group name="G4" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="Clean4">
                  <files><file source="c4/base.dat" destination="base/4.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="Twin4">
                  <files><file source="t4/base.dat" destination="base/4.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
        <installStep name="Step5">
          <optionalFileGroups>
            <group name="G5" type="SelectExactlyOne">
              <plugins order="Explicit">
                <plugin name="Plain5">
                  <files><file source="p5/base.dat" destination="base/5.dat"/></files>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
                <plugin name="Flag5">
                  <conditionFlags>
                    <flag name="cond">on</flag>
                  </conditionFlags>
                  <typeDescriptor><type name="Optional"/></typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
      <conditionalFileInstalls>
        <patterns>
          <pattern>
            <dependencies>
              <flagDependency flag="cond" value="on"/>
            </dependencies>
            <files>
              <file source="cfi/base5.dat" destination="base/5.dat"/>
              <file source="cfi/special.dat" destination="cond/special.dat"/>
            </files>
          </pattern>
        </patterns>
      </conditionalFileInstalls>
    </config>"#;

    let installer = parse_xml(xml, "");
    let atoms = build_atoms(&installer, DEFAULT_SIZE);
    let atom_index = build_atom_index(&atoms);

    // Target: six base files plus the conditional-only file. The exact
    // reproduction is Clean0 + Clean1..3 + (Clean4|Twin4) + Flag5. Flag5 sets
    // cond=on, firing the conditional install that supplies base/5.dat and
    // cond/special.dat, and Clean0 avoids decoy/extra.dat, so the tree matches
    // with no extras. Reaching it requires flipping both Step0 and Step5, which
    // single-flip local search cannot do (see the header comment).
    let target = build_target(
        &[
            "base/0.dat",
            "base/1.dat",
            "base/2.dat",
            "base/3.dat",
            "base/4.dat",
            "base/5.dat",
            "cond/special.dat",
        ],
        DEFAULT_SIZE,
    );
    let excluded = no_excluded();

    // Leave the conditional pattern Unknown: a pure flagDependency is evaluated
    // from the flag map regardless of the external override, so the conditional
    // fires exactly when Flag5 is selected.
    let overrides = InferenceOverrides {
        step_visible: vec![ExternalConditionOverride::Unknown; installer.steps.len()],
        conditional_active: vec![
            ExternalConditionOverride::Unknown;
            installer.conditional_patterns.len()
        ],
    };

    let propagation = propagate(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        &overrides,
        None,
    );

    let result = solve_fomod_csp(
        &installer,
        &atoms,
        &atom_index,
        &target,
        &excluded,
        Some(&overrides),
        Some(&propagation),
    );

    assert!(
        result.exact_match,
        "solver pruned the conditional-gated exact solution: missing={} extra={} size_mm={} hash_mm={}",
        result.missing, result.extra, result.size_mismatch, result.hash_mismatch
    );

    // Step5 is plugin order [Plain5, Flag5]; the exact solution must select
    // Flag5 (index 1) so the cond=on flag fires the conditional install.
    assert_eq!(result.selections.len(), 6);
    assert_eq!(result.selections[5].len(), 1);
    assert_eq!(result.selections[5][0].len(), 2);
    assert!(!result.selections[5][0][0]); // Plain5 not selected
    assert!(result.selections[5][0][1]); // Flag5 selected (sets cond=on)
}

/// Pins that a folder entry whose expansion would produce a top-level
/// `meta.ini` drops that atom while keeping the rest of the folder payload.
#[test]
fn expand_entry_folder_skips_archive_shipped_meta_ini() {
    // Some archives ship a meta.ini alongside their FOMOD payload. The target
    // scan (build_target_tree) excludes the installed meta.ini as MO2
    // metadata, so the expansion side must exclude it too - otherwise the
    // simulated output carries a file the target can never contain and
    // exact_match becomes unreachable (permanent "extra" plus fallback
    // penalty on an otherwise perfect reproduction).
    let folder = FomodFileEntry {
        is_folder: true,
        source: "opt".to_string(),
        destination: String::new(),
        priority: 0,
        ..FomodFileEntry::default()
    };

    let sorted_entries: Vec<String> = vec![
        "opt/meta.ini".to_string(),
        "opt/textures/blue.dds".to_string(),
    ];
    let entry_sizes: HashMap<String, u64> = HashMap::from([
        ("opt/meta.ini".to_string(), 10),
        ("opt/textures/blue.dds".to_string(), 100),
    ]);

    let mut out: Vec<FomodAtom> = Vec::new();
    expand_entry(
        &folder,
        &sorted_entries,
        &entry_sizes,
        0,
        Origin::Plugin,
        0,
        -1,
        &mut out,
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].dest_path, "textures/blue.dds");
}

/// Pins the single-file branch of `expand_entry`: a `<file>` entry whose
/// destination is the top-level `meta.ini` produces no atom at all. The target
/// scan (`build_target_tree`) drops the installed `meta.ini` as MO2 metadata,
/// so the expansion side must drop it too or `exact_match` becomes unreachable.
#[test]
fn expand_entry_file_skips_meta_ini_destination() {
    let file = FomodFileEntry {
        is_folder: false,
        source: "extras/meta.ini".to_string(),
        destination: "meta.ini".to_string(),
        priority: 0,
        ..FomodFileEntry::default()
    };

    let sorted_entries = vec!["extras/meta.ini".to_string()];
    let entry_sizes: HashMap<String, u64> = [("extras/meta.ini".to_string(), 10u64)]
        .into_iter()
        .collect();

    let mut out: Vec<FomodAtom> = Vec::new();
    expand_entry(
        &file,
        &sorted_entries,
        &entry_sizes,
        0,
        Origin::Required,
        -1,
        -1,
        &mut out,
    );

    assert!(out.is_empty());
}

/// Pins that only a TOP-LEVEL `meta.ini` destination is dropped as MO2
/// metadata: a nested one produced by folder expansion keeps flowing through
/// `expand_entry`.
#[test]
fn expand_entry_keeps_nested_meta_ini() {
    // Only the top-level meta.ini is MO2 metadata. A nested one (e.g.
    // skse/plugins/foo/meta.ini) is real payload and appears on both sides
    // of the diff, so it must keep flowing through expansion.
    let folder = FomodFileEntry {
        is_folder: true,
        source: "core".to_string(),
        destination: "skse".to_string(),
        priority: 0,
        ..FomodFileEntry::default()
    };

    let sorted_entries = vec!["core/plugins/meta.ini".to_string()];
    let entry_sizes: HashMap<String, u64> =
        HashMap::from([("core/plugins/meta.ini".to_string(), 10u64)]);

    let mut out: Vec<FomodAtom> = Vec::new();
    expand_entry(
        &folder,
        &sorted_entries,
        &entry_sizes,
        0,
        Origin::Plugin,
        0,
        -1,
        &mut out,
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].dest_path, "skse/plugins/meta.ini");
}
