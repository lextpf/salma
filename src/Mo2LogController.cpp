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
#include <utility>
#include <vector>

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
// Read [start, file_size) from log_path, drop any trailing partial line
// (bytes after the last '\n'), and parse complete lines from the kept
// region. Returns {lines, consumed_bytes} where consumed_bytes is the
// length of the kept region (so the caller can compute nextOffset =
// start + consumed_bytes). The unconsumed tail stays in the file and
// will be picked up by the next fetch once the writer flushes a newline.
//
// Without this trimming, std::getline returns the trailing partial line
// as if it were complete and the client receives a chopped-off last
// entry like "2026-04-27 11" with nextOffset already past the bytes,
// so the truncation persists across all subsequent fetches.
// ---------------------------------------------------------------------------
static std::pair<std::vector<std::string>, int64_t> read_complete_lines(const fs::path& log_path,
                                                                        int64_t start,
                                                                        int64_t file_size)
{
    std::vector<std::string> lines;
    if (start >= file_size)
        return {std::move(lines), 0};

    std::ifstream ifs(log_path, std::ios::binary);
    if (!ifs)
        return {std::move(lines), 0};
    ifs.seekg(start);

    auto to_read = static_cast<size_t>(file_size - start);
    std::string buf;
    buf.resize(to_read);
    ifs.read(buf.data(), static_cast<std::streamsize>(to_read));
    auto got = static_cast<size_t>(ifs.gcount());
    buf.resize(got);

    // Drop any bytes after the final '\n' in the read window.
    auto last_nl = buf.rfind('\n');
    if (last_nl == std::string::npos)
        return {std::move(lines), 0};

    size_t consumed = last_nl + 1;

    // Parse complete lines (each terminated by '\n' inside [0, consumed)).
    size_t line_begin = 0;
    for (size_t i = 0; i < consumed; ++i)
    {
        if (buf[i] == '\n')
        {
            size_t line_end = i;
            if (line_end > line_begin && buf[line_end - 1] == '\r')
                --line_end;
            lines.emplace_back(buf, line_begin, line_end - line_begin);
            line_begin = i + 1;
        }
    }

    return {std::move(lines), static_cast<int64_t>(consumed)};
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

    auto count_and_emit = [](const std::vector<std::string>& lines, int64_t next_offset)
    {
        json lines_arr = json::array();
        int errors = 0, warnings = 0, passes = 0;
        for (const auto& l : lines)
        {
            if (contains_keyword(l, "ERROR") || contains_keyword(l, "CRITICAL") ||
                contains_keyword(l, "FATAL") || contains_keyword(l, "FAIL"))
                errors++;
            else if (contains_keyword(l, "WARNING") || contains_keyword(l, "WARN"))
                warnings++;
            else if (contains_keyword(l, "PASS") || contains_keyword(l, "INFERRED"))
                passes++;
            lines_arr.push_back(l);
        }
        return json_response(200,
                             {{"lines", std::move(lines_arr)},
                              {"errors", errors},
                              {"warnings", warnings},
                              {"passes", passes},
                              {"nextOffset", next_offset}});
    };

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
            return count_and_emit({}, file_size);
        }

        auto [new_lines, consumed] = read_complete_lines(log_path, offset, file_size);

        // Apply max_lines limit: keep only the last max_lines entries
        // (max_lines == 0 means "all" - no trimming, consistent with full mode)
        if (max_lines > 0 && static_cast<int>(new_lines.size()) > max_lines)
        {
            new_lines.erase(new_lines.begin(),
                            new_lines.begin() + (static_cast<int>(new_lines.size()) - max_lines));
        }

        return count_and_emit(new_lines, offset + consumed);
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

    auto [lines, consumed] = read_complete_lines(log_path, read_start, file_size);
    return count_and_emit(lines, read_start + consumed);
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
