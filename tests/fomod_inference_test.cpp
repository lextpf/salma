#include <gtest/gtest.h>
#include <pugixml.hpp>

#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodDependencyEvaluator.h"
#include "FomodForwardSimulator.h"
#include "FomodIR.h"
#include "FomodIRParser.h"
#include "FomodPropagator.h"

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

// Helper: build AtomIndex from ExpandedAtoms.
static AtomIndex build_atom_index(const ExpandedAtoms& atoms)
{
    AtomIndex index;
    for (const auto& a : atoms.required)
        index[a.dest_path].push_back(a);
    for (const auto& plugin_atoms : atoms.per_plugin)
        for (const auto& a : plugin_atoms)
            index[a.dest_path].push_back(a);
    for (const auto& cond_atoms : atoms.per_conditional)
        for (const auto& a : cond_atoms)
            index[a.dest_path].push_back(a);
    return index;
}

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
    EXPECT_TRUE(domain[0]);    // Usable
    EXPECT_FALSE(domain[1]);   // NotUsable1
    EXPECT_FALSE(domain[2]);   // NotUsable2

    // CSP solver should also respect this: with ForceTrue, it should not widen.
    auto solver_result = solve_fomod_csp(installer, atoms, atom_index, target, excluded,
                                          &overrides, &result);
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
