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

namespace mo2server
{

// require alphabetic boundaries, so "PASSWORD" does not count as "PASS".
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

// keep one normal 10 MiB log readable while bounding response allocation.
static constexpr size_t kMaxLogReadChunk = size_t{12} * 1024 * 1024;

// read at most kMaxLogReadChunk bytes and commit only complete lines.
// consumed bytes exclude the trailing partial line so the next poll retries it.
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

    // retry this offset after the writer completes the line.
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

static crow::response read_log_file(const fs::path& log_path, const crow::request& req)
{
    // zero means all records; the byte limit remains the allocation guard.
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

    // count each record once, in error, warning, then pass order.
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

    // incremental mode reads forward from a byte offset.
    if (offset >= 0)
    {
        // an offset past EOF means the log was cleared or rotated.
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

        // retain the newest records; zero means all.
        if (max_lines > 0 && static_cast<int>(new_lines.size()) > max_lines)
        {
            new_lines.erase(new_lines.begin(),
                            new_lines.begin() + (static_cast<int>(new_lines.size()) - max_lines));
        }

        return count_and_emit(new_lines, offset + consumed);
    }

    // full mode reverse-seeks from EOF before reading forward.
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
                        // begin after this newline to return exactly max_lines.
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

crow::response Mo2Controller::get_logs(const crow::request& req)
{
    // use the logger's resolved path to match its write target.
    return read_log_file(mo2core::Logger::instance().log_path(), req);
}

crow::response Mo2Controller::get_test_logs(const crow::request& req)
{
    // test.log shares the executable directory with test_all.py.
    return read_log_file(mo2core::executable_directory() / "test.log", req);
}

crow::response Mo2Controller::clear_logs()
{
    // serialize truncation against the persistent writer.
    auto& logger = mo2core::Logger::instance();
    if (logger.clear_log())
    {
        logger.log("[server] Cleared logs/salma.log");
        return json_response(200, {{"success", true}});
    }
    return json_response(500, {{"error", "Failed to clear salma.log"}});
}

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
