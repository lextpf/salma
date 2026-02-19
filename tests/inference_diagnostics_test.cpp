#include <gtest/gtest.h>
#include <pugixml.hpp>

#include <nlohmann/json.hpp>

#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodInferenceAtoms.h"
#include "FomodIR.h"
#include "FomodIRParser.h"
#include "FomodPropagator.h"
#include "InferenceDiagnostics.h"

using namespace mo2core;

namespace
{

FomodInstaller parse_xml(const char* xml)
{
    pugi::xml_document doc;
    doc.load_string(xml);
    return FomodIRParser::parse(doc, "");
}

}  // namespace

// ---------------------------------------------------------------------------
// Schema v2 wire format
// ---------------------------------------------------------------------------

TEST(InferenceDiagnostics, AssembleJson_EmitsSchemaV2)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);

    SolverResult result;
    result.selections.resize(1);
    result.selections[0].resize(1);
    result.selections[0][0] = {true};
    result.exact_match = true;

    InferenceDiagnosticsBuilder builder(installer);
    builder.add_plugin_reason(0, 0, 0, ReasonCode::FORCED_SELECT_ALL, "SelectAll");
    builder.set_group_resolved_by(0, 0, "propagation.select_all");
    PropagationResult dummy_prop;
    builder.absorb_solver(result);
    builder.finalize(result, dummy_prop, installer);

    auto j = assemble_json(installer, result, builder.diagnostics());

    ASSERT_EQ(j["schema_version"], 2);
    ASSERT_TRUE(j["steps"].is_array());
    ASSERT_EQ(j["steps"].size(), 1);

    const auto& step = j["steps"][0];
    ASSERT_EQ(step["name"], "Step1");
    ASSERT_TRUE(step.contains("confidence"));
    ASSERT_TRUE(step["confidence"]["band"].is_string());
    ASSERT_TRUE(step.contains("visible"));

    const auto& group = step["groups"][0];
    ASSERT_EQ(group["resolved_by"], "propagation.select_all");
    ASSERT_TRUE(group["plugins"].is_array());
    ASSERT_EQ(group["plugins"].size(), 1);

    const auto& plugin = group["plugins"][0];
    ASSERT_EQ(plugin["name"], "P1");
    ASSERT_EQ(plugin["selected"], true);
    ASSERT_TRUE(plugin["confidence"]["composite"].is_number());
    ASSERT_TRUE(plugin["reasons"].is_array());
    ASSERT_FALSE(plugin["reasons"].empty());
    ASSERT_EQ(plugin["reasons"][0]["code"], "FORCED_SELECT_ALL");

    ASSERT_TRUE(j["diagnostics"].is_object());
    ASSERT_TRUE(j["diagnostics"]["timings_ms"].is_object());
    ASSERT_TRUE(j["diagnostics"]["repro"].is_object());
    ASSERT_TRUE(j["diagnostics"]["cache"].is_object());
    ASSERT_EQ(j["diagnostics"]["cache"]["hit"], false);
}

// ---------------------------------------------------------------------------
// ReasonCode -> string round trip
// ---------------------------------------------------------------------------

TEST(InferenceDiagnostics, ReasonCodeNames)
{
    EXPECT_EQ(reason_code_to_string(ReasonCode::IMPLICIT_DEFAULT), "IMPLICIT_DEFAULT");
    EXPECT_EQ(reason_code_to_string(ReasonCode::FORCED_REQUIRED), "FORCED_REQUIRED");
    EXPECT_EQ(reason_code_to_string(ReasonCode::UNIQUE_FILE_EVIDENCE), "UNIQUE_FILE_EVIDENCE");
    EXPECT_EQ(reason_code_to_string(ReasonCode::CSP_PHASE_GREEDY), "CSP_PHASE_GREEDY");
    EXPECT_EQ(reason_code_to_string(ReasonCode::FOMOD_PLUS_CACHE), "FOMOD_PLUS_CACHE");
}

// ---------------------------------------------------------------------------
// Confidence formula band thresholds
// ---------------------------------------------------------------------------

TEST(InferenceDiagnostics, ConfidenceBandThresholds)
{
    // Build a one-step installer so we can drive the formula end-to-end.
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);

    // Forced + exact match -> band high.
    {
        SolverResult result;
        result.selections.resize(1);
        result.selections[0].resize(1);
        result.selections[0][0] = {true};
        result.exact_match = true;

        InferenceDiagnosticsBuilder builder(installer);
        builder.add_plugin_reason(0, 0, 0, ReasonCode::FORCED_SELECT_ALL, "SelectAll");
        PropagationResult prop;
        builder.absorb_solver(result);
        builder.finalize(result, prop, installer);
        EXPECT_EQ(builder.diagnostics().run.confidence.band, "high");
    }

    // No reason + no exact match -> mid-range; CSP fallback path drops below high.
    {
        SolverResult result;
        result.selections.resize(1);
        result.selections[0].resize(1);
        result.selections[0][0] = {true};
        result.exact_match = false;
        result.phase_reached = "csp.fallback";
        result.phase_per_group.resize(1);
        result.phase_per_group[0].resize(1);
        result.phase_per_group[0][0] = "csp.fallback";

        InferenceDiagnosticsBuilder builder(installer);
        PropagationResult prop;
        builder.absorb_solver(result);
        builder.finalize(result, prop, installer);
        // Band must not be "high" - solver reached the global fallback.
        EXPECT_NE(builder.diagnostics().run.confidence.band, "high");
    }
}

// ---------------------------------------------------------------------------
// Cache-hit path produces FOMOD_PLUS_CACHE on every plugin
// ---------------------------------------------------------------------------

TEST(InferenceDiagnostics, CacheHit_AllPluginsCacheReason)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);

    SolverResult result;
    result.selections.resize(1);
    result.selections[0].resize(1);
    result.selections[0][0] = {true, false};

    InferenceDiagnosticsBuilder builder(installer);
    builder.set_cache_hit("fomod-plus");
    builder.absorb_solver(result);
    PropagationResult prop;
    builder.finalize(result, prop, installer);

    auto j = assemble_json(installer, result, builder.diagnostics());

    ASSERT_EQ(j["diagnostics"]["cache"]["hit"], true);
    ASSERT_EQ(j["diagnostics"]["cache"]["source"], "fomod-plus");
    ASSERT_EQ(j["diagnostics"]["phase_reached"], "tier1_cache");

    // Both selected and deselected plugins carry the cache reason.
    const auto& group = j["steps"][0]["groups"][0];
    ASSERT_EQ(group["plugins"].size(), 1);
    ASSERT_EQ(group["deselected"].size(), 1);

    auto has_cache_reason = [](const nlohmann::json& plugin)
    {
        for (const auto& r : plugin["reasons"])
        {
            if (r["code"] == "FOMOD_PLUS_CACHE")
            {
                return true;
            }
        }
        return false;
    };
    ASSERT_TRUE(has_cache_reason(group["plugins"][0]));
    ASSERT_TRUE(has_cache_reason(group["deselected"][0]));
}

// ---------------------------------------------------------------------------
// Propagator records FORCED_REQUIRED on Required plugins
// ---------------------------------------------------------------------------

TEST(InferenceDiagnostics, Propagator_RecordsForcedRequired)
{
    const char* xml = R"(
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
    </config>)";

    auto installer = parse_xml(xml);

    ExpandedAtoms atoms;
    int doc_order = 0;
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
                    atom.file_size = 100;
                    atom.origin = FomodAtom::Origin::Plugin;
                    plugin_atoms.push_back(atom);
                }
                atoms.per_plugin.push_back(std::move(plugin_atoms));
            }
        }
    }

    AtomIndex atom_index;
    for (const auto& plugin_atoms : atoms.per_plugin)
    {
        for (const auto& a : plugin_atoms)
        {
            atom_index[a.dest_path].push_back(a);
        }
    }

    TargetTree target;
    target["dx"] = TargetFile{100, 0};
    std::unordered_set<std::string> excluded;
    InferenceOverrides overrides;

    auto prop = propagate(installer, atoms, atom_index, target, excluded, overrides, nullptr);

    ASSERT_FALSE(prop.plugin_reasons.empty());
    int code = prop.plugin_reasons[0][0][0];
    EXPECT_EQ(code, static_cast<int>(ReasonCode::FORCED_REQUIRED));
}

// ---------------------------------------------------------------------------
// Backward-compat plugin reader (string and object forms)
// ---------------------------------------------------------------------------
//
// FomodService::read_plugin_name is a translation-unit-local helper, so we
// reproduce its behavior here. The intent of this test is to lock in the
// expected behavior of any consumer that adopts the same pattern.

namespace
{

std::string read_plugin_name_local(const nlohmann::json& entry)
{
    if (entry.is_string())
    {
        return entry.get<std::string>();
    }
    if (entry.is_object() && entry.contains("name") && entry["name"].is_string())
    {
        return entry["name"].get<std::string>();
    }
    return {};
}

}  // namespace

TEST(InferenceDiagnostics, BackwardCompatPluginReader)
{
    nlohmann::json string_form = "Plugin1";
    EXPECT_EQ(read_plugin_name_local(string_form), "Plugin1");

    nlohmann::json object_form;
    object_form["name"] = "Plugin1";
    object_form["selected"] = true;
    EXPECT_EQ(read_plugin_name_local(object_form), "Plugin1");

    nlohmann::json malformed = 42;
    EXPECT_EQ(read_plugin_name_local(malformed), "");
}
