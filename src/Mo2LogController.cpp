#include "Mo2Controller.h"
#include "Mo2Helpers.h"

#include "Logger.h"

#include <cctype>
#include <charconv>
#include <cstring>
#include <deque>
#include <filesystem>
#include <format>
#include <fstream>
#include <nlohmann/json.hpp>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

// Returns true if `word` appears in `line` as a standalone keyword
// (not embedded in a larger alphabetic token like "PASSWORD" matching "PASS").
static bool contains_keyword(const std::string& line, std::string_view word)
{
    std::size_t pos = 0;
    while ((pos = line.find(word, pos)) != std::string::npos)
    {
        bool left_ok = (pos == 0 || !std::isalpha(static_cast<unsigned char>(line[pos - 1])));
        bool right_ok = (pos + word.size() >= line.size() ||
                         !std::isalpha(static_cast<unsigned char>(line[pos + word.size()])));
        if (left_ok && right_ok)
            return true;
        pos += word.size();
    }
    return false;
}

// ---------------------------------------------------------------------------
// Shared log-reading helper (used by get_logs and get_test_logs)
// ---------------------------------------------------------------------------

static crow::response read_log_file(const fs::path& log_path, const crow::request& req)
{
    static constexpr int kMaxLinesLimit = 5000;
    int max_lines = 100;
    auto lines_param = req.url_params.get("lines");
    if (lines_param)
    {
        int parsed = 0;
        auto [ptr, ec] =
            std::from_chars(lines_param, lines_param + std::strlen(lines_param), parsed);
        if (ec == std::errc{})
        {
            max_lines = parsed;
        }
        if (max_lines < 0)
        {
            max_lines = 0;
        }
        if (max_lines > kMaxLinesLimit)
        {
            max_lines = kMaxLinesLimit;
        }
    }

    int64_t offset = -1;
    auto offset_param = req.url_params.get("offset");
    if (offset_param)
    {
        int64_t parsed_offset = 0;
        auto [ptr2, ec2] =
            std::from_chars(offset_param, offset_param + std::strlen(offset_param), parsed_offset);
        if (ec2 != std::errc{})
            offset = -1;
        else if (parsed_offset < 0)
            offset = -1;
        else
            offset = parsed_offset;
    }

    if (!fs::exists(log_path))
        return json_response(200,
                             {{"lines", json::array()},
                              {"errors", 0},
                              {"warnings", 0},
                              {"passes", 0},
                              {"nextOffset", 0}});

    auto file_size = static_cast<int64_t>(fs::file_size(log_path));

    // Incremental mode: seek to offset, read only new lines
    if (offset >= 0)
    {
        // File was truncated (log cleared) - tell client to reset
        if (offset > file_size)
        {
            return json_response(200,
                                 {{"lines", json::array()},
                                  {"errors", 0},
                                  {"warnings", 0},
                                  {"passes", 0},
                                  {"nextOffset", 0},
                                  {"reset", true}});
        }

        // No new data
        if (offset == file_size)
        {
            return json_response(200,
                                 {{"lines", json::array()},
                                  {"errors", 0},
                                  {"warnings", 0},
                                  {"passes", 0},
                                  {"nextOffset", file_size}});
        }

        std::ifstream ifs(log_path, std::ios::binary);
        ifs.seekg(offset);
        if (!ifs)
        {
            return json_response(200,
                                 {{"lines", json::array()},
                                  {"errors", 0},
                                  {"warnings", 0},
                                  {"passes", 0},
                                  {"nextOffset", 0},
                                  {"reset", true}});
        }
        std::deque<std::string> lines_deque;
        std::string line;
        while (std::getline(ifs, line))
        {
            // Strip trailing \r from Windows line endings
            if (!line.empty() && line.back() == '\r')
                line.pop_back();
            lines_deque.push_back(std::move(line));
        }

        // Apply max_lines limit: keep only the last max_lines entries
        if (max_lines >= 0 && static_cast<int>(lines_deque.size()) > max_lines)
        {
            lines_deque.erase(
                lines_deque.begin(),
                lines_deque.begin() + (static_cast<int>(lines_deque.size()) - max_lines));
        }

        json lines_arr = json::array();
        int errors = 0, warnings = 0, passes = 0;
        for (auto& l : lines_deque)
        {
            if (contains_keyword(l, "ERROR") || contains_keyword(l, "CRITICAL") ||
                contains_keyword(l, "FATAL") || contains_keyword(l, "FAIL"))
                errors++;
            else if (contains_keyword(l, "WARNING") || contains_keyword(l, "WARN"))
                warnings++;
            else if (contains_keyword(l, "PASS") || contains_keyword(l, "INFERRED"))
                passes++;
            lines_arr.push_back(std::move(l));
        }

        return json_response(200,
                             {{"lines", lines_arr},
                              {"errors", errors},
                              {"warnings", warnings},
                              {"passes", passes},
                              {"nextOffset", file_size}});
    }

    // Full mode: reverse-seek from EOF to find the last N newlines,
    // then read forward from that position. This is O(N * avg_line_len)
    // instead of O(file_size) for the deque-based approach.
    std::ifstream ifs(log_path, std::ios::binary);
    int64_t read_start = 0;
    if (max_lines > 0 && file_size > 0)
    {
        static constexpr int64_t kChunkSize = 8192;
        int newlines_found = 0;
        int64_t pos = file_size;
        while (pos > 0 && newlines_found <= max_lines)
        {
            int64_t chunk_start = std::max(static_cast<int64_t>(0), pos - kChunkSize);
            auto chunk_len = static_cast<std::streamsize>(pos - chunk_start);
            ifs.seekg(chunk_start);
            std::string chunk(static_cast<size_t>(chunk_len), '\0');
            ifs.read(chunk.data(), chunk_len);
            for (auto it = chunk.rbegin(); it != chunk.rend(); ++it)
            {
                if (*it == '\n')
                {
                    newlines_found++;
                    if (newlines_found > max_lines)
                    {
                        // Position just after this newline
                        read_start =
                            chunk_start + static_cast<int64_t>(std::distance(it, chunk.rend()));
                        break;
                    }
                }
            }
            pos = chunk_start;
        }
    }

    ifs.clear();
    ifs.seekg(read_start);
    json lines_arr = json::array();
    int errors = 0, warnings = 0, passes = 0;
    std::string line;
    while (std::getline(ifs, line))
    {
        if (!line.empty() && line.back() == '\r')
            line.pop_back();
        if (contains_keyword(line, "ERROR") || contains_keyword(line, "CRITICAL") ||
            contains_keyword(line, "FATAL") || contains_keyword(line, "FAIL"))
            errors++;
        else if (contains_keyword(line, "WARNING") || contains_keyword(line, "WARN"))
            warnings++;
        else if (contains_keyword(line, "PASS") || contains_keyword(line, "INFERRED"))
            passes++;
        lines_arr.push_back(std::move(line));
    }

    return json_response(200,
                         {{"lines", lines_arr},
                          {"errors", errors},
                          {"warnings", warnings},
                          {"passes", passes},
                          {"nextOffset", file_size}});
}

// ---------------------------------------------------------------------------
// GET /api/logs?lines=N&offset=B  (default lines=100)
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_logs(const crow::request& req)
{
    return read_log_file(fs::current_path() / "logs" / "salma.log", req);
}

// ---------------------------------------------------------------------------
// GET /api/logs/test?lines=N&offset=B  (default lines=100)
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_test_logs(const crow::request& req)
{
    return read_log_file(fs::current_path() / "test.log", req);
}

// ---------------------------------------------------------------------------
// POST /api/logs/clear
// ---------------------------------------------------------------------------

crow::response Mo2Controller::clear_logs()
{
    // Use Logger::clear_log() to coordinate truncation with the persistent
    // file handle, avoiding corrupted writes from concurrent log calls.
    auto& logger = mo2core::Logger::instance();
    if (logger.clear_log())
    {
        logger.log("[server] Cleared logs/salma.log");
        return json_response(200, {{"success", true}});
    }
    return json_response(500, {{"error", "Failed to clear salma.log"}});
}

// ---------------------------------------------------------------------------
// POST /api/logs/clear/test
// ---------------------------------------------------------------------------

crow::response Mo2Controller::clear_test_logs()
{
    auto log_path = fs::current_path() / "test.log";

    try
    {
        std::ofstream ofs(log_path, std::ios::trunc);
        if (!ofs)
        {
            return json_response(500, {{"error", "Failed to clear test.log"}});
        }
        mo2core::Logger::instance().log("[server] Cleared test.log");
        return json_response(200, {{"success", true}});
    }
    catch (const std::exception& ex)
    {
        return json_response(500,
                             {{"error", std::format("Failed to clear test.log: {}", ex.what())}});
    }
}

}  // namespace mo2server
