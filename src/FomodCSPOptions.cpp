#include "FomodCSPOptions.h"

#include "FomodCSPPrecompute.h"
#include "FomodDependencyEvaluator.h"
#include "Logger.h"

#include <algorithm>
#include <cassert>
#include <format>
#include <numeric>

namespace mo2core
{

bool is_exact_group_mode(int gidx, const std::unordered_set<int>* exact_groups)
{
    return exact_groups && exact_groups->count(gidx) > 0;
}

int effective_select_any_cap(int gidx,
                             int select_any_cap,
                             const std::unordered_set<int>* exact_groups)
{
    if (is_exact_group_mode(gidx, exact_groups))
        return kSelectAnyCapFull;
    return select_any_cap;
}

std::string group_name(const Precompute& pre, const GroupRef& g)
{
    return std::format("step {} \"{}\" / group {} \"{}\"",
                       g.step_idx,
                       pre.installer->steps[g.step_idx].name,
                       g.group_idx,
                       pre.installer->steps[g.step_idx].groups[g.group_idx].name);
}

static std::vector<GroupOption> generate_raw_options(
    const FomodGroup& group,
    const std::vector<int>& evidence,
    int flat_start,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap)
{
    const size_t n = group.plugins.size();
    if (n == 0)
        return {GroupOption{}};

    const size_t fs = static_cast<size_t>(flat_start);

    std::vector<bool> required(n, false);
    std::vector<bool> usable(n, false);
    std::vector<bool> dynamic(n, false);
    std::vector<bool> dynamic_external(n, false);
    for (size_t i = 0; i < n; ++i)
    {
        const auto& plugin = group.plugins[i];
        dynamic[i] = !plugin.type_patterns.empty();
        if (dynamic[i])
        {
            for (const auto& pattern : plugin.type_patterns)
            {
                if (condition_depends_on_external_state(pattern.condition))
                {
                    dynamic_external[i] = true;
                    break;
                }
            }
        }

        auto eff = evaluate_plugin_type(plugin, flags, nullptr);
        if (eff == PluginType::Required)
            required[i] = true;
        if (eff != PluginType::NotUsable)
            usable[i] = true;
    }
    bool has_required = std::any_of(required.begin(), required.end(), [](bool b) { return b; });
    bool has_dynamic_external =
        std::any_of(dynamic_external.begin(), dynamic_external.end(), [](bool b) { return b; });

    // External file/plugin/game checks are unknown during standalone inference.
    // Keep externally-dynamic plugins selectable and avoid forcing them as
    // Required so SelectExactlyOne groups don't collapse to arbitrary options.
    if (has_dynamic_external)
    {
        for (size_t i = 0; i < n; ++i)
        {
            if (dynamic_external[i])
                usable[i] = true;

            // Preserve Required for static/flag-only semantics.
            if (required[i] && dynamic_external[i])
            {
                required[i] = false;
            }
        }
        has_required = std::any_of(required.begin(), required.end(), [](bool b) { return b; });
    }

    std::vector<size_t> order(n);
    std::iota(order.begin(), order.end(), size_t{0});
    std::sort(order.begin(),
              order.end(),
              [&](size_t a, size_t b) { return evidence[fs + a] > evidence[fs + b]; });

    auto empty_option = [&]() { return GroupOption(n, false); };
    auto any_selected = [](const GroupOption& opt)
    { return std::any_of(opt.begin(), opt.end(), [](bool b) { return b; }); };

    std::vector<GroupOption> options;

    switch (group.type)
    {
        case FomodGroupType::SelectAll:
        {
            auto opt = empty_option();
            for (size_t i = 0; i < n; ++i)
                opt[i] = usable[i];
            options.push_back(std::move(opt));
            break;
        }
        case FomodGroupType::SelectExactlyOne:
        {
            if (has_required)
            {
                for (size_t i : order)
                {
                    if (!required[i])
                        continue;
                    auto opt = empty_option();
                    opt[i] = true;
                    options.push_back(std::move(opt));
                }
            }
            else
            {
                for (size_t i : order)
                {
                    if (!usable[i])
                        continue;
                    auto opt = empty_option();
                    opt[i] = true;
                    options.push_back(std::move(opt));
                }
            }
            break;
        }
        case FomodGroupType::SelectAtMostOne:
        {
            if (has_required)
            {
                for (size_t i : order)
                {
                    if (!required[i])
                        continue;
                    auto opt = empty_option();
                    opt[i] = true;
                    options.push_back(std::move(opt));
                }
            }
            else
            {
                for (size_t i : order)
                {
                    if (!usable[i])
                        continue;
                    auto opt = empty_option();
                    opt[i] = true;
                    options.push_back(std::move(opt));
                }
                options.push_back(empty_option());
            }
            break;
        }
        case FomodGroupType::SelectAtLeastOne:
        case FomodGroupType::SelectAny:
        {
            bool at_least_one = (group.type == FomodGroupType::SelectAtLeastOne);
            int positive_evidence = 0;
            for (size_t i = 0; i < n; ++i)
            {
                if (usable[i] && evidence[fs + i] > 0)
                    positive_evidence++;
            }

            // When SelectAny has no evidence signal, exhaustive powersets on medium-sized
            // groups blow up search space without improving reconstruction quality.
            bool force_heuristic_select_any =
                (select_any_cap > 0 && group.type == FomodGroupType::SelectAny && !has_required &&
                 positive_evidence == 0 && n >= 8);

            if (n <= 10 && !force_heuristic_select_any)
            {
                uint64_t limit = 1ULL << n;
                std::vector<std::pair<int, uint64_t>> scored;
                for (uint64_t mask = 0; mask < limit; ++mask)
                {
                    if (at_least_one && mask == 0)
                        continue;
                    bool valid = true;
                    for (size_t i = 0; i < n; ++i)
                    {
                        bool selected = (mask & (1ULL << i)) != 0;
                        if (required[i] && !selected)
                        {
                            valid = false;
                            break;
                        }
                        if (selected && !usable[i])
                        {
                            valid = false;
                            break;
                        }
                    }
                    if (!valid)
                        continue;

                    int score = 0;
                    for (size_t i = 0; i < n; ++i)
                        if ((mask & (1ULL << i)) != 0)
                            score += evidence[fs + i];
                    scored.emplace_back(score, mask);
                }

                std::sort(scored.begin(),
                          scored.end(),
                          [](const auto& a, const auto& b) { return a.first > b.first; });

                for (const auto& [_, mask] : scored)
                {
                    auto opt = empty_option();
                    for (size_t i = 0; i < n; ++i)
                        opt[i] = (mask & (1ULL << i)) != 0;
                    options.push_back(std::move(opt));
                }
            }
            else
            {
                auto greedy = empty_option();
                for (size_t i = 0; i < n; ++i)
                {
                    if (required[i])
                        greedy[i] = true;
                    if (usable[i] && evidence[fs + i] > 0)
                        greedy[i] = true;
                }
                if (!at_least_one || any_selected(greedy))
                    options.push_back(greedy);

                for (size_t i : order)
                {
                    if (!greedy[i] || required[i])
                        continue;
                    auto var = greedy;
                    var[i] = false;
                    if (at_least_one && !any_selected(var))
                        continue;
                    options.push_back(std::move(var));
                }

                for (size_t i : order)
                {
                    if (!usable[i])
                        continue;
                    auto opt = empty_option();
                    for (size_t j = 0; j < n; ++j)
                        opt[j] = required[j];
                    opt[i] = true;
                    options.push_back(std::move(opt));
                }

                if (!at_least_one)
                {
                    auto opt = empty_option();
                    for (size_t i = 0; i < n; ++i)
                        opt[i] = required[i];
                    options.push_back(std::move(opt));
                }

                // Large SelectAny groups often act as "filter" flag selectors.
                // Singletons can miss valid installs that require choosing
                // multiple filters, so keep a bounded set of pair candidates.
                bool allow_pair_candidates =
                    (group.type == FomodGroupType::SelectAny && positive_evidence > 0);
                if (allow_pair_candidates)
                {
                    std::vector<size_t> pair_candidates;
                    for (size_t i : order)
                    {
                        if (!usable[i] || required[i])
                            continue;
                        pair_candidates.push_back(i);
                    }

                    // Large SelectAny "filter" groups frequently need two
                    // simultaneous selectors. For small-to-medium groups keep
                    // the full pair neighborhood; only trim truly large ones.
                    constexpr size_t kFullPairLimit = 16;
                    constexpr size_t kHeuristicPairLimit = 8;
                    size_t pair_limit = (pair_candidates.size() <= kFullPairLimit)
                                            ? pair_candidates.size()
                                            : kHeuristicPairLimit;
                    if (pair_candidates.size() > pair_limit)
                        pair_candidates.resize(pair_limit);

                    for (size_t a = 0; a < pair_candidates.size(); ++a)
                    {
                        for (size_t b = a + 1; b < pair_candidates.size(); ++b)
                        {
                            auto opt = empty_option();
                            for (size_t j = 0; j < n; ++j)
                                opt[j] = required[j];
                            opt[pair_candidates[a]] = true;
                            opt[pair_candidates[b]] = true;
                            if (at_least_one && !any_selected(opt))
                                continue;
                            options.push_back(std::move(opt));
                        }
                    }
                }
            }
            break;
        }
    }

    // Enforce intra-group dependencyType consistency using flags set by the option itself.
    // This prevents invalid combinations such as selecting a "Merged" plugin together with
    // plugins that become NotUsable when that merged flag is present.
    std::erase_if(options,
                  [&](const GroupOption& opt)
                  {
                      auto local_flags = flags;
                      for (size_t i = 0; i < n; ++i)
                      {
                          if (!(i < opt.size() && opt[i]))
                              continue;
                          for (const auto& [fn, fv] : group.plugins[i].condition_flags)
                              local_flags[fn] = fv;
                      }

                      for (size_t i = 0; i < n; ++i)
                      {
                          if (!(i < opt.size() && opt[i]))
                              continue;
                          auto eff = evaluate_plugin_type(group.plugins[i], local_flags, nullptr);
                          if (eff == PluginType::NotUsable && !dynamic_external[i])
                              return true;
                      }
                      return false;
                  });

    if (options.empty())
    {
        auto opt = empty_option();
        for (size_t i = 0; i < n; ++i)
            opt[i] = required[i];
        options.push_back(std::move(opt));
    }

    return options;
}

static OptionProfile build_option_profile(const GroupRef& gref,
                                          const GroupOption& option,
                                          const Precompute& pre)
{
    OptionProfile p;
    p.option = option;

    std::unordered_set<std::string> useful;
    std::unordered_set<std::string> extra;

    const size_t pc = static_cast<size_t>(gref.plugin_count);
    const size_t fs = static_cast<size_t>(gref.flat_start);
    for (size_t pi = 0; pi < pc; ++pi)
    {
        if (!(pi < option.size() && option[pi]))
            continue;

        size_t flat_plugin = fs + pi;
        p.evidence_score += pre.evidence[flat_plugin];
        p.unique_support += pre.plugin_unique_support[flat_plugin];

        const auto& step = pre.installer->steps[gref.step_idx];
        const auto& group = step.groups[gref.group_idx];
        const auto& plugin = group.plugins[pi];

        for (const auto& [fn, fv] : plugin.condition_flags)
        {
            p.flags_written[fn] = fv;
            if (pre.needed_flags.count(fn))
                p.sets_needed_flag = true;
        }

        if (flat_plugin < pre.atoms->per_plugin.size())
        {
            for (const auto& atom : pre.atoms->per_plugin[flat_plugin])
            {
                if (pre.excluded->count(atom.dest_path))
                    continue;
                p.produced.insert(atom.dest_path);
                p.produced_atoms.insert(atom.dest_path + "|" + atom.source_path);
                if (pre.target->count(atom.dest_path))
                    useful.insert(atom.dest_path);
                else
                    extra.insert(atom.dest_path);
            }
        }
    }

    p.useful_dests = static_cast<int>(useful.size());
    p.extra_dests = static_cast<int>(extra.size());
    return p;
}

static uint64_t option_signature(const OptionProfile& p)
{
    uint64_t h = 14695981039346656037ULL;

    std::vector<std::string> produced_atoms(p.produced_atoms.begin(), p.produced_atoms.end());
    std::sort(produced_atoms.begin(), produced_atoms.end());
    for (const auto& atom_sig : produced_atoms)
        hash_combine(h, fnv1a_hash(atom_sig.data(), atom_sig.size()));

    std::vector<std::pair<std::string, std::string>> flags(p.flags_written.begin(),
                                                           p.flags_written.end());
    std::sort(flags.begin(), flags.end());
    for (const auto& [fn, fv] : flags)
    {
        hash_combine(h, fnv1a_hash(fn.data(), fn.size()));
        hash_combine(h, fnv1a_hash(fv.data(), fv.size()));
    }

    return h;
}

static bool better_equivalent_option(const OptionProfile& a, const OptionProfile& b)
{
    if (a.evidence_score != b.evidence_score)
        return a.evidence_score > b.evidence_score;
    if (a.unique_support != b.unique_support)
        return a.unique_support > b.unique_support;
    if (a.useful_dests != b.useful_dests)
        return a.useful_dests > b.useful_dests;
    return a.extra_dests < b.extra_dests;
}

static std::vector<GroupOption> reduce_options(const GroupRef& gref,
                                               const std::vector<GroupOption>& raw,
                                               const Precompute& pre,
                                               SolverStats& stats,
                                               int select_any_cap,
                                               bool exact_mode)
{
    if (raw.empty())
        return {};

    std::vector<OptionProfile> prof;
    prof.reserve(raw.size());
    for (const auto& opt : raw)
        prof.push_back(build_option_profile(gref, opt, pre));

    std::vector<size_t> keep;
    for (size_t i = 0; i < prof.size(); ++i)
    {
        if (!exact_mode && prof[i].extra_dests > 0 && prof[i].useful_dests == 0 &&
            !prof[i].sets_needed_flag)
        {
            stats.dropped_extra_only_options++;
            continue;
        }
        keep.push_back(i);
    }
    if (keep.empty())
    {
        keep.resize(prof.size());
        std::iota(keep.begin(), keep.end(), 0);
    }

    std::unordered_map<uint64_t, size_t> best_by_sig;
    for (size_t i : keep)
    {
        auto sig = option_signature(prof[i]);
        auto it = best_by_sig.find(sig);
        if (it == best_by_sig.end())
            best_by_sig[sig] = i;
        else
        {
            if (better_equivalent_option(prof[i], prof[it->second]))
                it->second = i;
            stats.collapsed_equivalent_options++;
        }
    }

    std::vector<size_t> candidates;
    for (const auto& [sig, idx] : best_by_sig)
        candidates.push_back(idx);

    // Keep all reduced candidates; unique support is used as ordering signal
    // but not as a hard force to avoid collapsing valid one-of choices.

    std::sort(candidates.begin(),
              candidates.end(),
              [&](size_t a, size_t b)
              {
                  if (prof[a].evidence_score != prof[b].evidence_score)
                      return prof[a].evidence_score > prof[b].evidence_score;
                  if (prof[a].unique_support != prof[b].unique_support)
                      return prof[a].unique_support > prof[b].unique_support;
                  if (prof[a].useful_dests != prof[b].useful_dests)
                      return prof[a].useful_dests > prof[b].useful_dests;
                  return prof[a].extra_dests < prof[b].extra_dests;
              });

    const auto& group = pre.installer->steps[gref.step_idx].groups[gref.group_idx];
    bool capped_select_any =
        (group.type == FomodGroupType::SelectAny || group.type == FomodGroupType::SelectAtLeastOne);
    if (capped_select_any && select_any_cap > 0 &&
        candidates.size() > static_cast<size_t>(select_any_cap))
    {
        std::vector<size_t> narrowed;
        narrowed.reserve(static_cast<size_t>(select_any_cap));

        std::vector<bool> chosen_rank(candidates.size(), false);
        auto pick_rank = [&](size_t rank)
        {
            if (rank >= candidates.size() || chosen_rank[rank] ||
                narrowed.size() >= static_cast<size_t>(select_any_cap))
                return;
            chosen_rank[rank] = true;
            narrowed.push_back(candidates[rank]);
        };

        // Keep the best option, then diversify by number of selected plugins.
        pick_rank(0);
        std::unordered_set<int> seen_selected_counts;
        for (size_t rank = 0;
             rank < candidates.size() && narrowed.size() < static_cast<size_t>(select_any_cap);
             ++rank)
        {
            const auto& opt = prof[candidates[rank]].option;
            int selected = static_cast<int>(std::count(opt.begin(), opt.end(), true));
            if (seen_selected_counts.insert(selected).second)
                pick_rank(rank);
        }

        for (size_t rank = 0;
             rank < candidates.size() && narrowed.size() < static_cast<size_t>(select_any_cap);
             ++rank)
        {
            pick_rank(rank);
        }

        stats.capped_select_any_options += static_cast<int>(candidates.size() - narrowed.size());
        candidates = std::move(narrowed);
    }

    std::vector<GroupOption> out;
    out.reserve(candidates.size());
    for (size_t i : candidates)
        out.push_back(prof[i].option);

    if (out.empty())
        out.push_back(raw[0]);
    return out;
}

const CachedOptions& get_options_for_group(
    int gidx,
    const Precompute& pre,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats)
{
    bool exact_mode = is_exact_group_mode(gidx, exact_groups);
    int effective_cap = effective_select_any_cap(gidx, select_any_cap, exact_groups);
    if (gidx < 0 || static_cast<size_t>(gidx) >= pre.group_cache_flags.size())
    {
        Logger::instance().log_error(std::format(
            "[solver] gidx {} out of range (size {})", gidx, pre.group_cache_flags.size()));
        static const CachedOptions kEmpty{};
        return kEmpty;
    }
    OptionCacheKey key{
        gidx, hash_flag_subset(flags, pre.group_cache_flags[gidx]), effective_cap, exact_mode};
    auto it = cache.find(key);
    if (it != cache.end())
        return it->second;

    const auto& gref = pre.groups[gidx];
    const auto& group = pre.installer->steps[gref.step_idx].groups[gref.group_idx];

    auto raw = generate_raw_options(group, pre.evidence, gref.flat_start, flags, effective_cap);

    // If propagation narrowed the domain, filter out options that select eliminated plugins.
    if (pre.propagation)
    {
        const auto& domains = pre.propagation->narrowed_domains;
        if (static_cast<size_t>(gref.step_idx) < domains.size() &&
            static_cast<size_t>(gref.group_idx) < domains[gref.step_idx].size())
        {
            const auto& group_domain = domains[gref.step_idx][gref.group_idx];
            std::erase_if(raw,
                          [&](const GroupOption& opt)
                          {
                              for (size_t i = 0; i < opt.size() && i < group_domain.size(); ++i)
                                  if (opt[i] && !group_domain[i])
                                      return true;
                              return false;
                          });
        }
    }

    auto reduced = reduce_options(gref, raw, pre, stats, effective_cap, exact_mode);

    CachedOptions entry;
    entry.options = std::move(reduced);
    entry.profiles.reserve(entry.options.size());
    for (const auto& opt : entry.options)
        entry.profiles.push_back(build_option_profile(gref, opt, pre));

    auto [ins, _] = cache.emplace(key, std::move(entry));
    if (!stats.logged_group_options[gidx])
    {
        stats.logged_group_options[gidx] = true;
        int positive_evidence = 0;
        int usable_plugins = 0;
        const size_t pc = static_cast<size_t>(gref.plugin_count);
        const size_t fs = static_cast<size_t>(gref.flat_start);
        for (size_t pi = 0; pi < pc; ++pi)
        {
            if (pre.evidence[fs + pi] > 0)
                positive_evidence++;
            auto eff = evaluate_plugin_type(group.plugins[pi], flags, nullptr);
            if (eff != PluginType::NotUsable)
                usable_plugins++;
        }
        Logger::instance().log(std::format("[solver] Branching {}: options raw={} reduced={}",
                                           group_name(pre, gref),
                                           raw.size(),
                                           ins->second.options.size()));
        Logger::instance().log(
            std::format("[solver] Group stats {}: plugins={}, usable={}, "
                        "positive_evidence={}",
                        group_name(pre, gref),
                        gref.plugin_count,
                        usable_plugins,
                        positive_evidence));
    }

    return ins->second;
}

}  // namespace mo2core
