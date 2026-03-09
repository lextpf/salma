#include <gtest/gtest.h>
#include <pugixml.hpp>

#include <algorithm>
#include <string>
#include <vector>

#include "FomodAtom.hpp"
#include "FomodCSPSolver.hpp"
#include "FomodDependencyEvaluator.hpp"
#include "FomodForwardSimulator.hpp"
#include "FomodInferenceAtoms.hpp"
#include "FomodInferenceService.hpp"
#include "FomodIR.hpp"
#include "FomodIRParser.hpp"
#include "FomodPropagator.hpp"

using namespace mo2core;

// Helper: parse XML string to FomodInstaller IR.
static FomodInstaller parse_xml(const char* xml, const std::string& prefix = "")
{
    pugi::xml_document doc;
    doc.load_string(xml);
    return FomodIRParser::parse(doc, prefix);
}

// Helper: build ExpandedAtoms from installer IR with synthetic atoms.
// Each non-auto file entry gets an atom with the given file_size.
static ExpandedAtoms build_atoms(const FomodInstaller& installer, uint64_t file_size = 100)
{
    ExpandedAtoms atoms;
    int doc_order = 0;

    for (const auto& fe : installer.required_files)
    {
        FomodAtom atom;
        atom.source_path = fe.source;
        atom.dest_path = fe.destination;
        atom.priority = fe.priority;
        atom.document_order = doc_order++;
        atom.file_size = file_size;
        atom.origin = FomodAtom::Origin::Required;
        atoms.required.push_back(atom);
    }

    int flat_idx = 0;
    for (const auto& step : installer.steps)
    {
        for (const auto& group : step.groups)
        {
            for (const auto& plugin : group.plugins)
            {
                std::vector<FomodAtom> plugin_atoms;
                for (const auto& fe : plugin.files)
                {
                    FomodAtom atom;
                    atom.source_path = fe.source;
                    atom.dest_path = fe.destination;
                    atom.priority = fe.priority;
                    atom.document_order = doc_order++;
                    atom.file_size = file_size;
                    atom.origin = FomodAtom::Origin::Plugin;
                    atom.plugin_index = flat_idx;
                    atom.always_install = fe.always_install;
                    atom.install_if_usable = fe.install_if_usable;
                    plugin_atoms.push_back(atom);
                }
                atoms.per_plugin.push_back(std::move(plugin_atoms));
                flat_idx++;
            }
        }
    }

    for (int ci = 0; ci < static_cast<int>(installer.conditional_patterns.size()); ++ci)
    {
        std::vector<FomodAtom> cond_atoms;
        for (const auto& fe : installer.conditional_patterns[ci].files)
        {
            FomodAtom atom;
            atom.source_path = fe.source;
            atom.dest_path = fe.destination;
            atom.priority = fe.priority;
            atom.document_order = doc_order++;
            atom.file_size = file_size;
            atom.origin = FomodAtom::Origin::Conditional;
            atom.conditional_index = ci;
            cond_atoms.push_back(atom);
        }
        atoms.per_conditional.push_back(std::move(cond_atoms));
    }

    return atoms;
}

// AtomIndex construction comes straight from the production helper
// (mo2core::build_atom_index in FomodInferenceAtoms.hpp).

// Helper: build a target tree from a set of dest paths.
static TargetTree build_target(const std::vector<std::string>& paths, uint64_t file_size = 100)
{
    TargetTree tree;
    for (const auto& p : paths)
        tree[p] = TargetFile{file_size, 0};
    return tree;
}

// ---------------------------------------------------------------------------
// Test 1: Propagator resolves SelectAll without CSP
// ---------------------------------------------------------------------------
TEST(FomodInference, SelectAll_Deterministic)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer);
    auto atom_index = build_atom_index(atoms);
    auto target = build_target({"textures/hi.dds", "textures/lo.dds", "textures/n.dds"});
    std::unordered_set<std::string> excluded;

    InferenceOverrides overrides;
    overrides.step_visible.assign(installer.steps.size(), ExternalConditionOverride::Unknown);
    overrides.conditional_active.assign(installer.conditional_patterns.size(),
                                        ExternalConditionOverride::Unknown);

    auto result = propagate(installer, atoms, atom_index, target, excluded, overrides, nullptr);

    ASSERT_TRUE(result.fully_resolved);
    ASSERT_EQ(result.resolved_groups.size(), 1u);

    // All 3 plugins should be selected (SelectAll).
    const auto& domain = result.narrowed_domains[0][0];
    ASSERT_EQ(domain.size(), 3u);
    EXPECT_TRUE(domain[0]);
    EXPECT_TRUE(domain[1]);
    EXPECT_TRUE(domain[2]);
}

// ---------------------------------------------------------------------------
// Test 2: evaluate_plugin_type returns first matching pattern's type
// ---------------------------------------------------------------------------
TEST(FomodInference, DependencyType_FirstPatternWins)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    ASSERT_EQ(installer.steps.size(), 1u);
    const auto& plugin = installer.steps[0].groups[0].plugins[0];

    // Both patterns match (same condition), first should win.
    std::unordered_map<std::string, std::string> flags = {{"mode", "advanced"}};
    auto eff_type = evaluate_plugin_type(plugin, flags, nullptr);
    EXPECT_EQ(eff_type, PluginType::Recommended);
}

// ---------------------------------------------------------------------------
// Test 3: simulate() installs files from all true conditional patterns
// ---------------------------------------------------------------------------
TEST(FomodInference, Conditional_AllMatchingPatternsApplied)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer);

    // Select P1 (step 0, group 0, plugin 0).
    std::vector<std::vector<std::vector<bool>>> selections = {{{true}}};

    InferenceOverrides overrides;
    overrides.step_visible.assign(1, ExternalConditionOverride::ForceTrue);
    overrides.conditional_active.assign(2, ExternalConditionOverride::ForceTrue);

    auto sim = simulate(installer, atoms, selections, nullptr, &overrides);

    EXPECT_TRUE(sim.files.count("base.esp"));
    EXPECT_TRUE(sim.files.count("patch_a.esp"));
    EXPECT_TRUE(sim.files.count("patch_b.esp"));
}

// ---------------------------------------------------------------------------
// Test 3b: output-tree data contract. The Files tab is built by
// FomodInferenceService::add_output_tree, which serializes simulate()'s file
// map sorted by destination path. This guards that every selected plugin file
// reaches the map and that the sort yields stable lexicographic order, carrying
// the size + source each entry renders.
// ---------------------------------------------------------------------------
TEST(FomodInference, OutputTree_SelectedFilesSortedByDest)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer, 100);

    // Select P1 (step 0, group 0, plugin 0).
    std::vector<std::vector<std::vector<bool>>> selections = {{{true}}};

    InferenceOverrides overrides;
    overrides.step_visible.assign(1, ExternalConditionOverride::ForceTrue);

    auto sim = simulate(installer, atoms, selections, nullptr, &overrides);

    // Every selected file reaches the simulated tree (the output-tree source).
    ASSERT_EQ(sim.files.size(), 3u);
    ASSERT_TRUE(sim.files.count("alpha.esp"));
    ASSERT_TRUE(sim.files.count("zebra.esp"));
    ASSERT_TRUE(sim.files.count("meshes/core.nif"));

    // Replicate add_output_tree's sort: entries ordered by destination path.
    std::vector<std::string> dests;
    for (const auto& [dest, atom] : sim.files)
    {
        dests.push_back(dest);
    }
    std::sort(dests.begin(), dests.end());

    EXPECT_EQ(dests[0], "alpha.esp");
    EXPECT_EQ(dests[1], "meshes/core.nif");
    EXPECT_EQ(dests[2], "zebra.esp");

    // Each winning atom carries the size + source the Files tab renders.
    EXPECT_EQ(sim.files.at("alpha.esp").file_size, 100u);
    EXPECT_EQ(sim.files.at("alpha.esp").source_path, "a/alpha.esp");
    EXPECT_EQ(sim.files.at("zebra.esp").source_path, "z/zebra.esp");
    EXPECT_EQ(sim.files.at("meshes/core.nif").source_path, "m/core.nif");
}

// ---------------------------------------------------------------------------
// Test 4: Parser orders steps alphabetically when no order attribute
// ---------------------------------------------------------------------------
TEST(FomodInference, Order_DefaultAscending)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    ASSERT_EQ(installer.steps.size(), 3u);
    EXPECT_EQ(installer.steps[0].name, "Alpha");
    EXPECT_EQ(installer.steps[1].name, "Mid");
    EXPECT_EQ(installer.steps[2].name, "Zeta");
}

// ---------------------------------------------------------------------------
// Test 5: Domain widening skipped when step override is ForceTrue
// ---------------------------------------------------------------------------
TEST(FomodInference, DomainWidening_RespectsEvidence)
{
    // Build a group with 3 plugins where type evaluation makes only 1 usable.
    // With ForceTrue step override, widening should NOT happen -- domain stays narrow.
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer);
    auto atom_index = build_atom_index(atoms);
    auto target = build_target({"u.esp"});
    std::unordered_set<std::string> excluded;

    // With ForceTrue override, propagation should resolve the single usable plugin.
    InferenceOverrides overrides;
    overrides.step_visible = {ExternalConditionOverride::ForceTrue};
    overrides.conditional_active = {};

    auto result = propagate(installer, atoms, atom_index, target, excluded, overrides, nullptr);
    ASSERT_TRUE(result.fully_resolved);

    const auto& domain = result.narrowed_domains[0][0];
    EXPECT_TRUE(domain[0]);   // Usable
    EXPECT_FALSE(domain[1]);  // NotUsable1
    EXPECT_FALSE(domain[2]);  // NotUsable2

    // CSP solver should also respect this: with ForceTrue, it should not widen.
    auto solver_result =
        solve_fomod_csp(installer, atoms, atom_index, target, excluded, &overrides, &result);
    // Only the usable plugin should be selected.
    ASSERT_EQ(solver_result.selections.size(), 1u);
    ASSERT_EQ(solver_result.selections[0].size(), 1u);
    ASSERT_EQ(solver_result.selections[0][0].size(), 3u);
    EXPECT_TRUE(solver_result.selections[0][0][0]);
    EXPECT_FALSE(solver_result.selections[0][0][1]);
    EXPECT_FALSE(solver_result.selections[0][0][2]);
}

// ---------------------------------------------------------------------------
// Test 6: Propagator eliminates plugins with no target evidence
// ---------------------------------------------------------------------------
TEST(FomodInference, PropagatorNarrowsDomains)
{
    // SelectExactlyOne group with 3 plugins, each with unique files.
    // Only plugin B's files are in the target.
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer);
    auto atom_index = build_atom_index(atoms);
    // Only plugin B's file is in the target.
    auto target = build_target({"textures/b.dds"});
    std::unordered_set<std::string> excluded;

    InferenceOverrides overrides;
    overrides.step_visible = {ExternalConditionOverride::Unknown};
    overrides.conditional_active = {};

    auto result = propagate(installer, atoms, atom_index, target, excluded, overrides, nullptr);

    // A and C should be eliminated (unique atoms miss target), B survives.
    // SelectExactlyOne with 1 usable => resolved.
    ASSERT_TRUE(result.fully_resolved);
    const auto& domain = result.narrowed_domains[0][0];
    EXPECT_FALSE(domain[0]);  // A eliminated
    EXPECT_TRUE(domain[1]);   // B survives
    EXPECT_FALSE(domain[2]);  // C eliminated
}

// ---------------------------------------------------------------------------
// Test 7: Propagator flag propagation resolves downstream groups
// ---------------------------------------------------------------------------
TEST(FomodInference, PropagatorFlagPropagation)
{
    // Step 1: SelectAll group with 1 plugin that sets flag "mode"="advanced".
    // Step 2: SelectExactlyOne group with 2 plugins:
    //   - Plugin A: dependencyType pattern checks flag mode=advanced => NotUsable
    //   - Plugin B: dependencyType pattern checks flag mode=advanced => Required
    // After resolving step 1, propagator should propagate flags and resolve step 2.
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer);
    auto atom_index = build_atom_index(atoms);
    auto target = build_target({"init.esp", "advanced.esp"});
    std::unordered_set<std::string> excluded;

    InferenceOverrides overrides;
    overrides.step_visible.assign(installer.steps.size(), ExternalConditionOverride::Unknown);
    overrides.conditional_active = {};

    auto result = propagate(installer, atoms, atom_index, target, excluded, overrides, nullptr);

    // Step 1: SelectAll resolves Init (iteration 1).
    // Flag mode=advanced propagates.
    // Step 2: BasicPatch becomes NotUsable, AdvancedPatch is only option => resolved (iteration 2).
    ASSERT_TRUE(result.fully_resolved);
    ASSERT_EQ(result.resolved_groups.size(), 2u);

    // Step 1, group 0: Init selected
    EXPECT_TRUE(result.narrowed_domains[0][0][0]);

    // Step 2, group 0: BasicPatch eliminated, AdvancedPatch selected
    EXPECT_FALSE(result.narrowed_domains[1][0][0]);
    EXPECT_TRUE(result.narrowed_domains[1][0][1]);
}

// ---------------------------------------------------------------------------
// Test: simulator's install_if_usable evaluation matches real installer's
// chronological flag order (regression for the CBBE 3BA exact=true vs
// real-test-fail divergence).
//
// Pre-Patch-A bug: simulate_into ran a global Phase 3 that evaluated each
// plugin's eff_type with flags accumulated through Phase 2 from EVERY selected
// plugin (including ones lexically after the plugin being evaluated). When P1
// has an install_if_usable atom whose dependencyType resolves to NotUsable
// only after a flag set by P2 (later), the simulator suppressed P1's atom.
// The real installer's enqueue_plugin_files queues P1's install_if_usable
// atom unconditionally for a SELECTED plugin.
// ---------------------------------------------------------------------------
TEST(FomodInference, Simulate_FlagOrderForInstallIfUsable_MatchesRealInstaller)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer);

    // Both P1 and P2 selected.
    std::vector<std::vector<std::vector<bool>>> selections = {{{true}}, {{true}}};

    InferenceOverrides overrides;
    overrides.step_visible.assign(installer.steps.size(), ExternalConditionOverride::ForceTrue);
    overrides.conditional_active.assign(installer.conditional_patterns.size(),
                                        ExternalConditionOverride::Unknown);

    auto sim = simulate(installer, atoms, selections, nullptr, &overrides);

    // P1's install_if_usable atom must be present even though P2 sets F=On.
    // The real installer queues P1's entry at step-1 enqueue time when F is
    // still unset (eff_type Optional), so x.dat ends up installed.
    EXPECT_TRUE(sim.files.count("x.dat")) << "install_if_usable atom from P1 was suppressed by "
                                             "flag set by a later plugin (P2)";
    EXPECT_TRUE(sim.files.count("y.dat"));
}

// ---------------------------------------------------------------------------
// Test: compute_overrides leaves duplicate-name conditional sibling steps as
// Unknown (regression for the Schlongs of Skyrim "all 7 steps ForceTrue" bug).
//
// Two installSteps both named "Skin Texture", each with a single plugin whose
// folder source produces the same dest paths. With the old "any atom hits
// target -> ForceTrue" rule, both steps got ForceTrue and the simulator ran
// them both, with last-applied content winning. The new rule requires at
// least one dest unique to the step.
// ---------------------------------------------------------------------------
TEST(FomodInference, Overrides_DuplicateNameStepsLeaveUnknown)
{
    FomodInstaller installer;

    // Two siblings with identical dest sets.
    for (int i = 0; i < 2; ++i)
    {
        FomodStep step;
        step.name = "Skin Texture";
        step.visible.emplace();
        step.visible->type = FomodConditionType::Flag;
        step.visible->flag_name = "BodyBuilder";
        step.visible->flag_value = (i == 0) ? "Off" : "On";

        FomodGroup group;
        group.name = "Skin Texture";
        group.type = FomodGroupType::SelectExactlyOne;

        FomodPlugin plugin;
        plugin.name = "Hairless";
        plugin.type = PluginType::Optional;

        step.groups.push_back(std::move(group));
        step.groups[0].plugins.push_back(std::move(plugin));
        installer.steps.push_back(std::move(step));
    }

    // Two atoms per step, all sharing the same dest set.
    ExpandedAtoms atoms;
    atoms.per_plugin.resize(2);
    auto make_atom = [](const std::string& src, const std::string& dst, int doc)
    {
        FomodAtom a;
        a.source_path = src;
        a.dest_path = dst;
        a.document_order = doc;
        a.file_size = 100;
        a.origin = FomodAtom::Origin::Plugin;
        return a;
    };
    atoms.per_plugin[0].push_back(make_atom("off/textures/body.dds", "textures/body.dds", 0));
    atoms.per_plugin[0].push_back(make_atom("off/textures/face.dds", "textures/face.dds", 1));
    atoms.per_plugin[1].push_back(make_atom("on/textures/body.dds", "textures/body.dds", 2));
    atoms.per_plugin[1].push_back(make_atom("on/textures/face.dds", "textures/face.dds", 3));

    AtomIndex idx = build_atom_index(atoms);
    auto target = build_target({"textures/body.dds", "textures/face.dds"});
    std::unordered_set<std::string> excluded;

    auto overrides =
        FomodInferenceService::compute_overrides(installer, atoms, idx, target, excluded);

    ASSERT_EQ(overrides.step_visible.size(), 2u);
    EXPECT_EQ(overrides.step_visible[0], ExternalConditionOverride::Unknown);
    EXPECT_EQ(overrides.step_visible[1], ExternalConditionOverride::Unknown);
}

// ---------------------------------------------------------------------------
// Test: compute_overrides forces step visibility True when the step has at
// least one dest unique to it (regression guard for the simple case).
// ---------------------------------------------------------------------------
TEST(FomodInference, Overrides_UniqueDestForcesTrue)
{
    FomodInstaller installer;
    FomodStep step;
    step.name = "Step1";

    FomodGroup group;
    group.name = "G1";
    group.type = FomodGroupType::SelectExactlyOne;

    FomodPlugin plugin;
    plugin.name = "P1";
    plugin.type = PluginType::Optional;

    step.groups.push_back(std::move(group));
    step.groups[0].plugins.push_back(std::move(plugin));
    installer.steps.push_back(std::move(step));

    ExpandedAtoms atoms;
    atoms.per_plugin.resize(1);
    FomodAtom a;
    a.source_path = "src/file.dat";
    a.dest_path = "file.dat";
    a.document_order = 0;
    a.file_size = 100;
    a.origin = FomodAtom::Origin::Plugin;
    atoms.per_plugin[0].push_back(a);

    AtomIndex idx = build_atom_index(atoms);
    auto target = build_target({"file.dat"});
    std::unordered_set<std::string> excluded;

    auto overrides =
        FomodInferenceService::compute_overrides(installer, atoms, idx, target, excluded);

    ASSERT_EQ(overrides.step_visible.size(), 1u);
    EXPECT_EQ(overrides.step_visible[0], ExternalConditionOverride::ForceTrue);
}

// ---------------------------------------------------------------------------
// Regression: branch-and-bound lower bound must treat a conditional-only dest
// as still fixable while a flag-setter group remains unassigned (finding 2.2,
// conditional-only dest admissibility).
//
// The lower bound's dest_to_groups / dest_to_size_match_groups /
// dest_to_hash_capable_groups maps are populated from Plugin-origin atoms only.
// A dest that some path produces solely via a conditionalFileInstalls pattern
// (gated on a flag set by a LATER group) has no entry there, so the buggy bound
// counts it as an unfixable miss and cannot_beat() prunes the subtree that
// would have reached the exact solution.
//
// This test must exercise the backtracking DFS, where lower_bound() runs. The
// scenario is therefore built as a genuine local optimum that greedy + local
// search CANNOT escape with a single group flip, so the solver is forced past
// Phase-1 into the global backtrack pass where the bound fires.
//
// Scenario (6 SelectExactlyOne groups, steps Step0..Step5):
//   - Step0: "Decoy0" directly produces the conditional dest "cond/special.dat"
//     AND an unwanted "decoy/extra.dat"; "Clean0" produces only "base/0.dat".
//     Decoy0 out-scores Clean0 (it uniquely supplies cond/special.dat), so
//     greedy picks Decoy0 -> missing=0, extra=1 (the decoy file).
//   - Step5 (LAST group, highest order position): "Plain5" directly produces
//     "base/5.dat" and "Flag5" produces no files but sets flag cond=on. The
//     conditionalFileInstalls pattern gated on cond=on produces BOTH
//     "base/5.dat" and "cond/special.dat". Because Plain5 is the unique plugin
//     producer of base/5.dat (evidence 3) it out-scores Flag5 (evidence 2 from
//     the conditional flag link), so greedy leaves the flag OFF and picks
//     Plain5. cond/special.dat is then supplied only by Decoy0.
//
// The greedy result {Decoy0, Clean1..4, Plain5} is a STRICT local optimum at
// {missing:0, extra:1} that no single flip improves:
//   - Flip Step0 Decoy0 -> Clean0: drops decoy/extra.dat but also drops
//     cond/special.dat (conditional still OFF because Plain5 is selected) ->
//     {missing:1, extra:0}, not better.
//   - Flip Step5 Plain5 -> Flag5: fires the conditional (cond/special.dat now
//     redundant, base/5.dat now from the pattern) but leaves decoy/extra.dat ->
//     {missing:0, extra:1}, not better.
// The exact reproduction needs BOTH changes at once (Clean0 AND Flag5), a
// 2-coordinated move that hill-climbing cannot reach. targeted_repair only
// touches SelectAny/AtLeastOne groups, so Phase-1 gives up here and the solver
// escalates to the global backtracking DFS.
//
// Why the buggy bound prunes the exact path: the bound fires at order position 4
// (group G4). On the prefix that chose Clean0 (not Decoy0) with Step5 still at
// its best value Plain5, "cond/special.dat" is missing from the partial sim.
// Its only dest_to_groups entry is Step0's Decoy0, already past in the order, so
// has_remaining_group() is false; the conditional's flag setter (Step5, position
// 5) is invisible to dest_to_groups. The buggy can_fix_missing reports it
// unfixable -> lb.missing = 1 > best.missing = 0 -> cannot_beat() prunes before
// Step5=Flag5 is ever tried, so exact_match stays false on revert.
//
// The fix's conditional_repair_remaining() sees that "cond/special.dat" is a
// conditional dest, that flag "cond" is needed, and that its setter group
// (Step5) is still unassigned (order_pos >= next_idx), so it treats the dest as
// fixable -> lb.missing = 0 -> the subtree survives and the exact is found.
//
// Step3 carries a second decoy "Swap3" that produces cond/special.dat instead
// of base/3.dat (it survives propagation because it only ever supplies a target
// file). Greedy prefers Clean3 (the unique producer of base/3.dat), but the DFS
// still explores the Swap3 branch inside the Decoy0 subtree. On that branch
// base/3.dat is missing and its only producer group (Step3) is already past in
// the order, so the bound prunes it on genuine, non-conditional grounds. This
// guarantees lower_bound > 0 in the pruning summary even WITH the fix present,
// proving the DFS bound path actually executed.
TEST(FomodInference, ConditionalDest_FlagSetByLaterGroup_ReachesExact)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);
    auto atoms = build_atoms(installer);
    auto atom_index = build_atom_index(atoms);

    // Target: six base files plus the conditional-only file. The exact
    // reproduction is Clean0 + Clean1..3 + (Clean4|Twin4) + Flag5. Flag5 sets
    // cond=on, firing the conditional install that supplies base/5.dat and
    // cond/special.dat, and Clean0 avoids decoy/extra.dat, so the tree matches
    // with no extras. Reaching it requires flipping both Step0 and Step5, which
    // single-flip local search cannot do (see the header comment).
    auto target = build_target({"base/0.dat",
                                "base/1.dat",
                                "base/2.dat",
                                "base/3.dat",
                                "base/4.dat",
                                "base/5.dat",
                                "cond/special.dat"});
    std::unordered_set<std::string> excluded;

    InferenceOverrides overrides;
    overrides.step_visible.assign(installer.steps.size(), ExternalConditionOverride::Unknown);
    // Leave the conditional pattern Unknown: a pure flagDependency is evaluated
    // from the flag map regardless of the external override, so the conditional
    // fires exactly when Flag5 is selected.
    overrides.conditional_active.assign(installer.conditional_patterns.size(),
                                        ExternalConditionOverride::Unknown);

    auto propagation =
        propagate(installer, atoms, atom_index, target, excluded, overrides, nullptr);

    auto result =
        solve_fomod_csp(installer, atoms, atom_index, target, excluded, &overrides, &propagation);

    EXPECT_TRUE(result.exact_match)
        << "solver pruned the conditional-gated exact solution: "
           "missing="
        << result.missing << " extra=" << result.extra << " size_mm=" << result.size_mismatch
        << " hash_mm=" << result.hash_mismatch;

    // Step5 is plugin order [Plain5, Flag5]; the exact solution must select
    // Flag5 (index 1) so the cond=on flag fires the conditional install.
    ASSERT_EQ(result.selections.size(), 6u);
    ASSERT_EQ(result.selections[5].size(), 1u);
    ASSERT_EQ(result.selections[5][0].size(), 2u);
    EXPECT_FALSE(result.selections[5][0][0]);  // Plain5 not selected
    EXPECT_TRUE(result.selections[5][0][1]);   // Flag5 selected (sets cond=on)
}

// ---------------------------------------------------------------------------
// Archive-shipped meta.ini is never FOMOD payload
// ---------------------------------------------------------------------------

TEST(FomodInference, ExpandEntry_FolderSkipsArchiveShippedMetaIni)
{
    // Some archives ship a meta.ini alongside their FOMOD payload. The target
    // scan (build_target_tree) excludes the installed meta.ini as MO2
    // metadata, so the expansion side must exclude it too - otherwise the
    // simulated output carries a file the target can never contain and
    // exact_match becomes unreachable (permanent "extra" plus fallback
    // penalty on an otherwise perfect reproduction).
    FomodFileEntry folder;
    folder.is_folder = true;
    folder.source = "opt";
    folder.destination = "";
    folder.priority = 0;

    std::vector<std::string> sorted_entries = {"opt/meta.ini", "opt/textures/blue.dds"};
    std::unordered_map<std::string, uint64_t> entry_sizes = {{"opt/meta.ini", 10},
                                                             {"opt/textures/blue.dds", 100}};

    std::vector<FomodAtom> out;
    expand_entry(folder, sorted_entries, entry_sizes, 0, FomodAtom::Origin::Plugin, 0, -1, out);

    ASSERT_EQ(out.size(), 1u);
    EXPECT_EQ(out[0].dest_path, "textures/blue.dds");
}

TEST(FomodInference, ExpandEntry_FileSkipsMetaIniDestination)
{
    FomodFileEntry file;
    file.is_folder = false;
    file.source = "extras/meta.ini";
    file.destination = "meta.ini";
    file.priority = 0;

    std::vector<std::string> sorted_entries = {"extras/meta.ini"};
    std::unordered_map<std::string, uint64_t> entry_sizes = {{"extras/meta.ini", 10}};

    std::vector<FomodAtom> out;
    expand_entry(file, sorted_entries, entry_sizes, 0, FomodAtom::Origin::Required, -1, -1, out);

    EXPECT_TRUE(out.empty());
}

TEST(FomodInference, ExpandEntry_KeepsNestedMetaIni)
{
    // Only the top-level meta.ini is MO2 metadata. A nested one (e.g.
    // skse/plugins/foo/meta.ini) is real payload and appears on both sides
    // of the diff, so it must keep flowing through expansion.
    FomodFileEntry folder;
    folder.is_folder = true;
    folder.source = "core";
    folder.destination = "skse";
    folder.priority = 0;

    std::vector<std::string> sorted_entries = {"core/plugins/meta.ini"};
    std::unordered_map<std::string, uint64_t> entry_sizes = {{"core/plugins/meta.ini", 10}};

    std::vector<FomodAtom> out;
    expand_entry(folder, sorted_entries, entry_sizes, 0, FomodAtom::Origin::Plugin, 0, -1, out);

    ASSERT_EQ(out.size(), 1u);
    EXPECT_EQ(out[0].dest_path, "skse/plugins/meta.ini");
}
