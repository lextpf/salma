#include "FomodForwardSimulator.h"
#include "FomodCSPSolver.h"
#include "FomodDependencyEvaluator.h"

namespace mo2core
{

// An atom overwrites the existing entry if it has >= priority.
// Since we process atoms in increasing document_order, same priority means
// the new atom came later in XML and should win (last-writer-wins semantics).
static bool should_overwrite(const FomodAtom& existing, const FomodAtom& new_atom)
{
    return new_atom.priority >= existing.priority;
}

static void apply_atom(SimulatedTree& tree, const FomodAtom& atom)
{
    auto it = tree.files.find(atom.dest_path);
    if (it == tree.files.end() || should_overwrite(it->second, atom))
    {
        tree.files.insert_or_assign(atom.dest_path, atom);
    }
}

void simulate_into(SimulatedTree& tree,
                   const FomodInstaller& installer,
                   const ExpandedAtoms& atoms,
                   const std::vector<std::vector<std::vector<bool>>>& selections,
                   const FomodDependencyContext* context,
                   const InferenceOverrides* overrides)
{
    tree.files.clear();
    std::unordered_map<std::string, std::string> flags;

    // Shared helpers to avoid duplicating step-visibility and flat_idx advancement.
    auto compute_step_visibility = [&](size_t si, const FomodStep& step) -> bool
    {
        if (!step.visible)
        {
            return true;
        }
        if (overrides && si < overrides->step_visible.size())
        {
            return evaluate_condition_inferred(
                *step.visible, flags, overrides->step_visible[si], context);
        }
        return evaluate_condition(*step.visible, flags, context);
    };

    // Phase 1: Required files
    for (const auto& atom : atoms.required)
    {
        apply_atom(tree, atom);
    }

    // Phase 2 (chronological pass for selected/Required plugins).
    // Mirrors FomodService::process_optional_files's Pass 1 + per-step Required
    // catch-all. For each plugin in step/group/plugin order, in a visible step:
    //   - eff_type is evaluated against the flag state at this plugin's position
    //     (matches the real installer, which detects Required-type plugins per
    //     step using flags accumulated up to that step).
    //   - If selected (via CSP) or Required-typed, accumulate condition_flags
    //     then apply ALL of this plugin's atoms unconditionally. The real
    //     installer's enqueue_plugin_files queues every entry without an
    //     eff_type filter, so for selected plugins install_if_usable atoms
    //     apply regardless of eff_type.
    // Unselected non-Required plugins are deferred to Phase 3 (their auto
    // atoms get evaluated against the FINAL flag state, like Pass 3 of the
    // real installer).
    std::vector<bool> processed_in_phase2(atoms.per_plugin.size(), false);
    int flat_idx = 0;
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];
        bool visible = compute_step_visibility(si, step);

        if (!visible)
        {
            // Skip all groups in this invisible step, but still advance flat_idx
            for (const auto& group : step.groups)
                flat_idx += static_cast<int>(group.plugins.size());
            continue;
        }

        for (size_t gi = 0; gi < step.groups.size(); ++gi)
        {
            const auto& group = step.groups[gi];
            for (size_t pi = 0; pi < group.plugins.size(); ++pi)
            {
                const auto& plugin = group.plugins[pi];
                bool selected = false;
                if (si < selections.size() && gi < selections[si].size() &&
                    pi < selections[si][gi].size())
                {
                    selected = selections[si][gi][pi];
                }

                if (!selected)
                {
                    auto eff_type = evaluate_plugin_type(plugin, flags, context);
                    if (eff_type == PluginType::Required)
                        selected = true;
                }

                if (selected && flat_idx < static_cast<int>(atoms.per_plugin.size()))
                {
                    for (const auto& [name, value] : plugin.condition_flags)
                    {
                        flags[name] = value;
                    }
                    for (const auto& atom : atoms.per_plugin[flat_idx])
                    {
                        apply_atom(tree, atom);
                    }
                    processed_in_phase2[flat_idx] = true;
                }
                flat_idx++;
            }
        }
    }

    // Phase 3 (final-flag-state pass for unselected non-Required plugins).
    // Mirrors FomodService::process_optional_files's Pass 3: for each plugin
    // not already processed, in a visible step, evaluate eff_type against the
    // FINAL flag state and apply auto atoms accordingly. always_install runs
    // unconditionally; install_if_usable runs only when eff_type != NotUsable.
    flat_idx = 0;
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];
        bool visible = compute_step_visibility(si, step);

        if (!visible)
        {
            for (const auto& group : step.groups)
                flat_idx += static_cast<int>(group.plugins.size());
            continue;
        }

        for (const auto& group : step.groups)
        {
            for (const auto& plugin : group.plugins)
            {
                if (flat_idx >= static_cast<int>(atoms.per_plugin.size()) ||
                    processed_in_phase2[flat_idx])
                {
                    flat_idx++;
                    continue;
                }
                auto eff_type = evaluate_plugin_type(plugin, flags, context);
                for (const auto& atom : atoms.per_plugin[flat_idx])
                {
                    if (atom.always_install)
                    {
                        apply_atom(tree, atom);
                    }
                    else if (atom.install_if_usable && eff_type != PluginType::NotUsable)
                    {
                        apply_atom(tree, atom);
                    }
                }
                flat_idx++;
            }
        }
    }

    // Phase 4: Conditional file installs
    for (size_t ci = 0; ci < installer.conditional_patterns.size(); ++ci)
    {
        const auto& pattern = installer.conditional_patterns[ci];
        bool active;
        if (overrides && ci < overrides->conditional_active.size())
        {
            active = evaluate_condition_inferred(
                pattern.condition, flags, overrides->conditional_active[ci], context);
        }
        else
        {
            active = evaluate_condition(pattern.condition, flags, context);
        }
        if (active)
        {
            if (ci < atoms.per_conditional.size())
            {
                for (const auto& atom : atoms.per_conditional[ci])
                {
                    apply_atom(tree, atom);
                }
            }
        }
    }
}

SimulatedTree simulate(const FomodInstaller& installer,
                       const ExpandedAtoms& atoms,
                       const std::vector<std::vector<std::vector<bool>>>& selections,
                       const FomodDependencyContext* context,
                       const InferenceOverrides* overrides)
{
    SimulatedTree tree;
    simulate_into(tree, installer, atoms, selections, context, overrides);
    return tree;
}

}  // namespace mo2core
