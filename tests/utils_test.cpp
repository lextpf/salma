#include <gtest/gtest.h>

#include <filesystem>

#include "Utils.h"

// --- to_lower ---

TEST(ToLower, LowercasesUpperCase)
{
    EXPECT_EQ(mo2core::to_lower("HELLO"), "hello");
}

TEST(ToLower, PreservesAlreadyLowerCase)
{
    EXPECT_EQ(mo2core::to_lower("hello"), "hello");
}

TEST(ToLower, MixedCase)
{
    EXPECT_EQ(mo2core::to_lower("HeLLo WoRLd"), "hello world");
}

TEST(ToLower, EmptyString)
{
    EXPECT_EQ(mo2core::to_lower(""), "");
}

TEST(ToLower, NonAlpha)
{
    EXPECT_EQ(mo2core::to_lower("123!@#"), "123!@#");
}

// --- normalize_path ---

TEST(NormalizePath, BackslashToForwardSlash)
{
    EXPECT_EQ(mo2core::normalize_path("textures\\lod\\file.dds"), "textures/lod/file.dds");
}

TEST(NormalizePath, StripsLeadingDotSlash)
{
    EXPECT_EQ(mo2core::normalize_path("./textures/lod/file.dds"), "textures/lod/file.dds");
}

TEST(NormalizePath, StripsLeadingSlash)
{
    EXPECT_EQ(mo2core::normalize_path("/textures/lod/file.dds"), "textures/lod/file.dds");
}

TEST(NormalizePath, StripsTrailingSlash)
{
    EXPECT_EQ(mo2core::normalize_path("textures/lod/"), "textures/lod");
}

TEST(NormalizePath, CollapsesDoubleSlash)
{
    EXPECT_EQ(mo2core::normalize_path("textures//lod//file.dds"), "textures/lod/file.dds");
}

TEST(NormalizePath, LowercasesPath)
{
    EXPECT_EQ(mo2core::normalize_path("Textures\\LOD\\File.DDS"), "textures/lod/file.dds");
}

TEST(NormalizePath, EmptyString)
{
    EXPECT_EQ(mo2core::normalize_path(""), "");
}

TEST(NormalizePath, MultipleDotSlashPrefixes)
{
    EXPECT_EQ(mo2core::normalize_path("././textures/file.dds"), "textures/file.dds");
}

TEST(NormalizePath, OnlySlashes)
{
    EXPECT_EQ(mo2core::normalize_path("///"), "");
}

TEST(NormalizePath, MixedSeparatorsAndPrefixes)
{
    EXPECT_EQ(mo2core::normalize_path("./\\Textures\\\\LOD/"), "textures/lod");
}

// --- random_hex_string ---

TEST(RandomHexString, DefaultLength)
{
    auto s = mo2core::random_hex_string();
    EXPECT_EQ(s.size(), 12u);
}

TEST(RandomHexString, CustomLength)
{
    auto s = mo2core::random_hex_string(32);
    EXPECT_EQ(s.size(), 32u);
}

TEST(RandomHexString, ZeroLength)
{
    auto s = mo2core::random_hex_string(0);
    EXPECT_TRUE(s.empty());
}

TEST(RandomHexString, OnlyHexChars)
{
    auto s = mo2core::random_hex_string(100);
    for (char c : s)
    {
        EXPECT_TRUE((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')) << "Non-hex char: " << c;
    }
}

TEST(RandomHexString, TwoCallsProduceDifferentResults)
{
    // Statistically near-impossible to collide at length 32
    auto a = mo2core::random_hex_string(32);
    auto b = mo2core::random_hex_string(32);
    EXPECT_NE(a, b);
}

// --- get_ordered_nodes ---

class GetOrderedNodesTest : public ::testing::Test
{
protected:
    pugi::xml_document doc;
};

TEST_F(GetOrderedNodesTest, AscendingOrder)
{
    doc.load_string(R"(<root order="Ascending">
        <item name="Charlie"/>
        <item name="Alice"/>
        <item name="Bob"/>
    </root>)");
    auto nodes = mo2core::get_ordered_nodes(doc.child("root"), "item");
    ASSERT_EQ(nodes.size(), 3u);
    EXPECT_STREQ(nodes[0].attribute("name").as_string(), "Alice");
    EXPECT_STREQ(nodes[1].attribute("name").as_string(), "Bob");
    EXPECT_STREQ(nodes[2].attribute("name").as_string(), "Charlie");
}

TEST_F(GetOrderedNodesTest, DescendingOrder)
{
    doc.load_string(R"(<root order="Descending">
        <item name="Alice"/>
        <item name="Charlie"/>
        <item name="Bob"/>
    </root>)");
    auto nodes = mo2core::get_ordered_nodes(doc.child("root"), "item");
    ASSERT_EQ(nodes.size(), 3u);
    EXPECT_STREQ(nodes[0].attribute("name").as_string(), "Charlie");
    EXPECT_STREQ(nodes[1].attribute("name").as_string(), "Bob");
    EXPECT_STREQ(nodes[2].attribute("name").as_string(), "Alice");
}

TEST_F(GetOrderedNodesTest, ExplicitOrder)
{
    doc.load_string(R"(<root order="Explicit">
        <item name="Charlie"/>
        <item name="Alice"/>
        <item name="Bob"/>
    </root>)");
    auto nodes = mo2core::get_ordered_nodes(doc.child("root"), "item");
    ASSERT_EQ(nodes.size(), 3u);
    EXPECT_STREQ(nodes[0].attribute("name").as_string(), "Charlie");
    EXPECT_STREQ(nodes[1].attribute("name").as_string(), "Alice");
    EXPECT_STREQ(nodes[2].attribute("name").as_string(), "Bob");
}

TEST_F(GetOrderedNodesTest, DefaultIsAscending)
{
    doc.load_string(R"(<root>
        <item name="Charlie"/>
        <item name="Alice"/>
        <item name="Bob"/>
    </root>)");
    auto nodes = mo2core::get_ordered_nodes(doc.child("root"), "item");
    ASSERT_EQ(nodes.size(), 3u);
    EXPECT_STREQ(nodes[0].attribute("name").as_string(), "Alice");
}

TEST_F(GetOrderedNodesTest, EmptyParent)
{
    doc.load_string(R"(<root order="Ascending"></root>)");
    auto nodes = mo2core::get_ordered_nodes(doc.child("root"), "item");
    EXPECT_TRUE(nodes.empty());
}

// --- xml_bool_attribute_true ---

class XmlBoolAttributeTest : public ::testing::Test
{
protected:
    pugi::xml_document doc;
};

TEST_F(XmlBoolAttributeTest, TrueString)
{
    doc.load_string(R"(<el val="true"/>)");
    EXPECT_TRUE(mo2core::xml_bool_attribute_true(doc.child("el").attribute("val")));
}

TEST_F(XmlBoolAttributeTest, TrueUpperCase)
{
    doc.load_string(R"(<el val="TRUE"/>)");
    EXPECT_TRUE(mo2core::xml_bool_attribute_true(doc.child("el").attribute("val")));
}

TEST_F(XmlBoolAttributeTest, One)
{
    doc.load_string(R"(<el val="1"/>)");
    EXPECT_TRUE(mo2core::xml_bool_attribute_true(doc.child("el").attribute("val")));
}

TEST_F(XmlBoolAttributeTest, FalseString)
{
    doc.load_string(R"(<el val="false"/>)");
    EXPECT_FALSE(mo2core::xml_bool_attribute_true(doc.child("el").attribute("val")));
}

TEST_F(XmlBoolAttributeTest, Zero)
{
    doc.load_string(R"(<el val="0"/>)");
    EXPECT_FALSE(mo2core::xml_bool_attribute_true(doc.child("el").attribute("val")));
}

TEST_F(XmlBoolAttributeTest, MissingAttribute)
{
    doc.load_string(R"(<el/>)");
    EXPECT_FALSE(mo2core::xml_bool_attribute_true(doc.child("el").attribute("val")));
}

// --- parse_plugin_type_string ---

TEST(ParsePluginTypeString, Required)
{
    EXPECT_EQ(mo2core::parse_plugin_type_string("Required"), mo2core::PluginType::Required);
}

TEST(ParsePluginTypeString, Recommended)
{
    EXPECT_EQ(mo2core::parse_plugin_type_string("Recommended"), mo2core::PluginType::Recommended);
}

TEST(ParsePluginTypeString, Optional)
{
    EXPECT_EQ(mo2core::parse_plugin_type_string("Optional"), mo2core::PluginType::Optional);
}

TEST(ParsePluginTypeString, NotUsable)
{
    EXPECT_EQ(mo2core::parse_plugin_type_string("NotUsable"), mo2core::PluginType::NotUsable);
}

TEST(ParsePluginTypeString, CouldBeUsable)
{
    EXPECT_EQ(mo2core::parse_plugin_type_string("CouldBeUsable"),
              mo2core::PluginType::CouldBeUsable);
}

TEST(ParsePluginTypeString, UnknownDefaultsToOptional)
{
    EXPECT_EQ(mo2core::parse_plugin_type_string("Bogus"), mo2core::PluginType::Optional);
}

TEST(ParsePluginTypeString, EmptyDefaultsToOptional)
{
    EXPECT_EQ(mo2core::parse_plugin_type_string(""), mo2core::PluginType::Optional);
}

// --- is_safe_mod_name ---

TEST(IsSafeModName, RejectsTraversalForward)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("../outside"));
}

TEST(IsSafeModName, RejectsTraversalBackward)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("..\\outside"));
}

TEST(IsSafeModName, RejectsBareDoubleDot)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name(".."));
}

TEST(IsSafeModName, RejectsAbsoluteWindowsPath)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("C:\\temp\\evil"));
}

TEST(IsSafeModName, RejectsDriveRoot)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("C:\\"));
}

TEST(IsSafeModName, RejectsLeadingSlash)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("/etc/passwd"));
}

TEST(IsSafeModName, RejectsForwardSlashInside)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("foo/bar"));
}

TEST(IsSafeModName, RejectsBackslashInside)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("foo\\bar"));
}

TEST(IsSafeModName, RejectsEmpty)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name(""));
}

TEST(IsSafeModName, RejectsWhitespaceOnly)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("   "));
}

TEST(IsSafeModName, RejectsBareDot)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("."));
}

TEST(IsSafeModName, RejectsTrailingDot)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("MyMod."));
}

TEST(IsSafeModName, RejectsTrailingSpace)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("MyMod "));
}

TEST(IsSafeModName, RejectsLeadingSpace)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name(" MyMod"));
}

TEST(IsSafeModName, RejectsWindowsReservedCon)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("CON"));
    EXPECT_FALSE(mo2core::is_safe_mod_name("con"));
    EXPECT_FALSE(mo2core::is_safe_mod_name("CON.txt"));
}

TEST(IsSafeModName, RejectsWindowsReservedComLpt)
{
    EXPECT_FALSE(mo2core::is_safe_mod_name("COM1"));
    EXPECT_FALSE(mo2core::is_safe_mod_name("LPT9"));
    EXPECT_FALSE(mo2core::is_safe_mod_name("nul"));
}

TEST(IsSafeModName, AcceptsSimpleName)
{
    EXPECT_TRUE(mo2core::is_safe_mod_name("SkyUI"));
}

TEST(IsSafeModName, AcceptsNameWithSpaces)
{
    EXPECT_TRUE(mo2core::is_safe_mod_name("My Mod 1.2"));
}

TEST(IsSafeModName, AcceptsDotsInside)
{
    EXPECT_TRUE(mo2core::is_safe_mod_name("A.B.C"));
}

TEST(IsSafeModName, AcceptsHyphensAndUnderscores)
{
    EXPECT_TRUE(mo2core::is_safe_mod_name("My_Cool-Mod"));
}

// Containment invariant: even if a hostile name slipped past
// is_safe_mod_name, the defense-in-depth is_inside check on the joined
// path catches it. This exercises the integration the upload controller
// relies on.
TEST(IsInsideRejectsModNameTraversal, GeneratedPathStaysInsideModsDir)
{
    namespace fs = std::filesystem;
    auto tmp = fs::temp_directory_path() / "salma_modname_containment_test";
    fs::create_directories(tmp);
    EXPECT_TRUE(mo2core::is_inside(tmp, tmp / "SkyUI"));
    EXPECT_FALSE(mo2core::is_inside(tmp, tmp / ".." / "escape"));
    fs::remove_all(tmp);
}
