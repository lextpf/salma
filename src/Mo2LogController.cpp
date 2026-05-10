#include "Mo2Controller.hpp"
#include "Mo2Helpers.hpp"

#include "Logger.hpp"
#include "Utils.hpp"

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

// Mo2LogController - the log-fetch protocol behind /api/logs and
// /api/logs/test, plus the two clear endpoints.
//
// One handler, read_log_file, serves both logs; they differ only in path.
// salma.log comes from Logger::log_path(), test.log from <exe dir>/test.log.
//
// Two modes, selected by the `offset` query parameter:
//
//   offset >= 0   incremental. Read forward from that byte position.
//   offset absent, negative, or unparseable
//                 full. Reverse-seek from EOF for the last `lines` newlines,
//                 then read forward from there.
//
// Every response carries lines, errors, warnings, passes and nextOffset; a
// reset flag appears only in the cleared-log case. An offset is a byte position
// in the file, never a line number.
//
//   client                          server                        salma.log
//   ------                          ------                        ---------
//   GET ?offset=0        --->  read [0, min(size, 12 MiB))    [A\nB\nC\nD-par
//                              drop bytes after the last \n              ^^^^^
//                                                                        held
//                   <---  lines=[A,B,C]  nextOffset=<byte after C\n>
//   GET ?offset=<after C> --->  nothing complete yet
//                   <---  lines=[]  nextOffset=<after C>, unchanged
//   ...the writer flushes the rest of D and a newline...
//   GET ?offset=<after C> --->
//                   <---  lines=[D]  nextOffset=<byte after D\n>
//
//   offset > file_size  ->  the log was cleared or rotated under the client:
//                           reply reset=true, nextOffset=0, no lines. The
//                           client must restart from 0.
//   offset == file_size ->  no new data; nextOffset is echoed back.
//
// The partial trailing line is held back deliberately. Returning it would
// report a half-written record as complete and advance nextOffset past those
// bytes, so the truncation would persist in every later fetch.
//
// `lines` caps how many entries a response carries: default 100, 0 means all,
// hard cap 500000, and an unparseable value falls back to the default. Its
// effect differs by mode. In full mode it picks where the read begins, by
// reverse-seeking that many newlines back from EOF. In incremental mode it is
// applied after the read, keeping the newest entries, so it does not bound the
// read at all. kMaxLogReadChunk is the only bound on how much is allocated.

namespace mo2server
{

// True when `word` appears in `line` as a standalone keyword. The alphabetic
// boundary check keeps "PASSWORD" from counting as a "PASS".
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

// Hard ceiling on the bytes one response may read. It sits above the 10 MiB
// rotation cap in Logger.cpp, so a whole un-rotated log fits in one response
// instead of silently starting partway through. Above the rotation size it is
// an allocation guard: a client asking for offset=0 against a larger file
// cannot make the handler buffer all of it.
static constexpr size_t kMaxLogReadChunk = size_t{12} * 1024 * 1024;

// ---------------------------------------------------------------------------
// Read [start, min(file_size, start + kMaxLogReadChunk)) from log_path, drop
// any trailing partial line (the bytes after the last '\n'), and split the kept
// region into lines. A trailing '\r' is removed from each line, so a CRLF log
// reads the same as an LF one.
//
// Returns {lines, consumed_bytes}. consumed_bytes is the length of the kept
// region, so the caller computes nextOffset = start + consumed_bytes. It is 0,
// with no lines, when start is at or past file_size, when the file cannot be
// opened, and when the window holds no newline at all. The unconsumed tail
// stays in the file and the next fetch picks it up once the writer has flushed
// a newline.
//
// Drop the trimming and a plain read hands back the trailing partial line as if
// it were complete: the client gets a chopped entry such as "2026-04-27 11"
// while nextOffset already points past those bytes, so the truncation persists
// in every later fetch.
//
// A single line longer than kMaxLogReadChunk stalls the incremental protocol,
// because no newline falls inside the window and consumed_bytes stays 0. Logger
// writes one bounded record per line, so that case does not arise today.
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

    auto to_read = std::min(static_cast<size_t>(file_size - start), kMaxLogReadChunk);
    std::string buf;
    buf.resize(to_read);
    ifs.read(buf.data(), static_cast<std::streamsize>(to_read));
    auto got = static_cast<size_t>(ifs.gcount());
    buf.resize(got);

    // No newline in the window means no complete line; report zero consumed so
    // the caller re-reads from the same offset next time.
    auto last_nl = buf.rfind('\n');
    if (last_nl == std::string::npos)
        return {std::move(lines), 0};

    size_t consumed = last_nl + 1;

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
// Implements both modes of the protocol at the top of this file, for get_logs
// and get_test_logs. Always answers 200. A missing file reports an empty log
// with nextOffset 0 rather than a 404, because no log yet is a normal state
// before anything has been written.
// ---------------------------------------------------------------------------

static crow::response read_log_file(const fs::path& log_path, const crow::request& req)
{
    // The dashboard asks for the whole log, not a tail sample, so this ceiling
    // is high on purpose. Rotation at 10 MiB keeps "all of it" bounded, and
    // kMaxLogReadChunk above, not this line count, is the real guard on how much
    // one response can allocate.
    static constexpr int kMaxLinesLimit = 500000;
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

    // Each line lands in at most one bucket. The chain is exclusive and ordered
    // error, then warning, then pass, so a line matching several keywords counts
    // only toward the first. The counts cover this response's lines, not the
    // whole file, so the dashboard accumulates them across incremental fetches.
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

    // Incremental mode.
    if (offset >= 0)
    {
        // Past EOF means the log was cleared or rotated under the client.
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

        if (offset == file_size)
        {
            return count_and_emit({}, file_size);
        }

        auto [new_lines, consumed] = read_complete_lines(log_path, offset, file_size);

        // Trim to the newest max_lines entries. 0 means all, matching full mode.
        if (max_lines > 0 && static_cast<int>(new_lines.size()) > max_lines)
        {
            new_lines.erase(new_lines.begin(),
                            new_lines.begin() + (static_cast<int>(new_lines.size()) - max_lines));
        }

        return count_and_emit(new_lines, offset + consumed);
    }

    // Full mode. Reverse-seek from EOF for the last max_lines newlines, then
    // read forward from there. Cost is proportional to the bytes those lines
    // occupy, not to the file size.
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
                        // Start just after this newline, so the count of
                        // returned lines is exactly max_lines.
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
    // Ask Logger for the path instead of rebuilding it, so the read cannot
    // drift from where the writes go.
    return read_log_file(mo2core::Logger::instance().log_path(), req);
}

// ---------------------------------------------------------------------------
// GET /api/logs/test?lines=N&offset=B  (default lines=100)
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_test_logs(const crow::request& req)
{
    // test.log lives next to test_all.py, which itself lives next to the exe.
    return read_log_file(mo2core::executable_directory() / "test.log", req);
}

// ---------------------------------------------------------------------------
// POST /api/logs/clear
// ---------------------------------------------------------------------------

crow::response Mo2Controller::clear_logs()
{
    // Truncating through Logger, not through the filesystem, so the truncate is
    // serialized against the persistent write handle. Truncating behind
    // Logger's back would corrupt a concurrent write.
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
    auto log_path = mo2core::executable_directory() / "test.log";

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
