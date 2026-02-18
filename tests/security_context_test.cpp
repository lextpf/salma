#include <gtest/gtest.h>

#include "SecurityContext.h"

#include <algorithm>

// --- default_allowed_origins ---

TEST(DefaultAllowedOrigins, IncludesProductionAndViteOrigins)
{
    auto defaults = mo2core::default_allowed_origins();
    EXPECT_EQ(defaults.size(), 4u);
    EXPECT_NE(std::ranges::find(defaults, "http://localhost:5000"), defaults.end());
    EXPECT_NE(std::ranges::find(defaults, "http://127.0.0.1:5000"), defaults.end());
    EXPECT_NE(std::ranges::find(defaults, "http://localhost:3000"), defaults.end());
    EXPECT_NE(std::ranges::find(defaults, "http://127.0.0.1:3000"), defaults.end());
}

// --- parse_origin_list ---

TEST(ParseOriginList, EmptyInputReturnsEmpty)
{
    EXPECT_TRUE(mo2core::parse_origin_list("").empty());
}

TEST(ParseOriginList, WhitespaceOnlyReturnsEmpty)
{
    EXPECT_TRUE(mo2core::parse_origin_list("   \t  ").empty());
}

TEST(ParseOriginList, SingleEntry)
{
    auto out = mo2core::parse_origin_list("http://localhost:5000");
    ASSERT_EQ(out.size(), 1u);
    EXPECT_EQ(out[0], "http://localhost:5000");
}

TEST(ParseOriginList, MultipleEntriesTrimsWhitespace)
{
    auto out = mo2core::parse_origin_list(
        "http://localhost:5000 ,  http://example.test  , http://127.0.0.1:3000");
    ASSERT_EQ(out.size(), 3u);
    EXPECT_EQ(out[0], "http://localhost:5000");
    EXPECT_EQ(out[1], "http://example.test");
    EXPECT_EQ(out[2], "http://127.0.0.1:3000");
}

TEST(ParseOriginList, DropsEmptyEntries)
{
    auto out = mo2core::parse_origin_list(",http://a,,http://b,,,");
    ASSERT_EQ(out.size(), 2u);
    EXPECT_EQ(out[0], "http://a");
    EXPECT_EQ(out[1], "http://b");
}

// --- origin_in_allowlist ---

TEST(OriginInAllowlist, MatchesExact)
{
    std::vector<std::string> allow{"http://localhost:5000", "http://127.0.0.1:5000"};
    EXPECT_TRUE(mo2core::origin_in_allowlist(allow, "http://localhost:5000"));
    EXPECT_TRUE(mo2core::origin_in_allowlist(allow, "http://127.0.0.1:5000"));
}

TEST(OriginInAllowlist, RejectsCaseMismatch)
{
    // Browsers send Origin lowercase per spec. Allowlist comparison is exact;
    // a mixed-case Origin must not match a lowercase allowlist entry.
    std::vector<std::string> allow{"http://localhost:5000"};
    EXPECT_FALSE(mo2core::origin_in_allowlist(allow, "http://LocalHost:5000"));
}

TEST(OriginInAllowlist, RejectsPortMismatch)
{
    std::vector<std::string> allow{"http://localhost:5000"};
    EXPECT_FALSE(mo2core::origin_in_allowlist(allow, "http://localhost:5001"));
}

TEST(OriginInAllowlist, RejectsSchemeMismatch)
{
    std::vector<std::string> allow{"http://localhost:5000"};
    EXPECT_FALSE(mo2core::origin_in_allowlist(allow, "https://localhost:5000"));
}

TEST(OriginInAllowlist, EmptyOriginRejected)
{
    std::vector<std::string> allow{"http://localhost:5000"};
    EXPECT_FALSE(mo2core::origin_in_allowlist(allow, ""));
}

TEST(OriginInAllowlist, EmptyAllowlistRejectsEverything)
{
    std::vector<std::string> allow;
    EXPECT_FALSE(mo2core::origin_in_allowlist(allow, "http://localhost:5000"));
}

// --- is_state_changing ---

TEST(IsStateChanging, PostPutDeletePatchAreStateChanging)
{
    EXPECT_TRUE(mo2core::is_state_changing("POST"));
    EXPECT_TRUE(mo2core::is_state_changing("PUT"));
    EXPECT_TRUE(mo2core::is_state_changing("DELETE"));
    EXPECT_TRUE(mo2core::is_state_changing("PATCH"));
}

TEST(IsStateChanging, GetHeadOptionsAreSafe)
{
    EXPECT_FALSE(mo2core::is_state_changing("GET"));
    EXPECT_FALSE(mo2core::is_state_changing("HEAD"));
    EXPECT_FALSE(mo2core::is_state_changing("OPTIONS"));
}

TEST(IsStateChanging, CaseInsensitive)
{
    EXPECT_TRUE(mo2core::is_state_changing("post"));
    EXPECT_TRUE(mo2core::is_state_changing("Put"));
    EXPECT_FALSE(mo2core::is_state_changing("get"));
}

TEST(IsStateChanging, UnknownMethodIsSafe)
{
    EXPECT_FALSE(mo2core::is_state_changing(""));
    EXPECT_FALSE(mo2core::is_state_changing("FROBNICATE"));
}

// --- constant_time_equals ---

TEST(ConstantTimeEquals, EqualStringsReturnTrue)
{
    EXPECT_TRUE(mo2core::constant_time_equals("hello", "hello"));
    EXPECT_TRUE(mo2core::constant_time_equals("", ""));
}

TEST(ConstantTimeEquals, DifferentStringsReturnFalse)
{
    EXPECT_FALSE(mo2core::constant_time_equals("hello", "world"));
    EXPECT_FALSE(mo2core::constant_time_equals("hello", "Hello"));
}

TEST(ConstantTimeEquals, DifferentLengthsReturnFalse)
{
    EXPECT_FALSE(mo2core::constant_time_equals("a", "ab"));
    EXPECT_FALSE(mo2core::constant_time_equals("abc", "ab"));
    EXPECT_FALSE(mo2core::constant_time_equals("", "x"));
}

TEST(ConstantTimeEquals, BinarySafeOnNonAscii)
{
    std::string a("\x00\xff\x7f", 3);
    std::string b("\x00\xff\x7f", 3);
    std::string c("\x00\xff\x7e", 3);
    EXPECT_TRUE(mo2core::constant_time_equals(a, b));
    EXPECT_FALSE(mo2core::constant_time_equals(a, c));
}

// --- SecurityContext ---

TEST(SecurityContextSingleton, TokenIs64HexChars)
{
    const auto& token = mo2core::SecurityContext::instance().csrf_token();
    EXPECT_EQ(token.size(), 64u);
    for (char c : token)
    {
        const unsigned char uc = static_cast<unsigned char>(c);
        const bool is_hex = (uc >= '0' && uc <= '9') || (uc >= 'a' && uc <= 'f');
        EXPECT_TRUE(is_hex) << "non-hex char in token: " << c;
    }
}

TEST(SecurityContextSingleton, IsOriginAllowedDelegates)
{
    auto& ctx = mo2core::SecurityContext::instance();
    // The default singleton has the four default origins (assuming
    // SALMA_ALLOWED_ORIGINS is not set in the test environment).
    if (ctx.allowed_origins() == mo2core::default_allowed_origins())
    {
        EXPECT_TRUE(ctx.is_origin_allowed("http://localhost:5000"));
        EXPECT_FALSE(ctx.is_origin_allowed("http://evil.example"));
        EXPECT_FALSE(ctx.is_origin_allowed(""));
    }
    else
    {
        // SALMA_ALLOWED_ORIGINS overrode defaults; just verify the
        // delegation path against whatever is loaded.
        for (const auto& origin : ctx.allowed_origins())
        {
            EXPECT_TRUE(ctx.is_origin_allowed(origin));
        }
        EXPECT_FALSE(ctx.is_origin_allowed("http://definitely-not-in-allowlist.example"));
    }
}

TEST(SecurityContextSingleton, InstanceIsStable)
{
    auto& a = mo2core::SecurityContext::instance();
    auto& b = mo2core::SecurityContext::instance();
    EXPECT_EQ(&a, &b);
    EXPECT_EQ(a.csrf_token(), b.csrf_token());
}
