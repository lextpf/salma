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

SimulatedTree simulate(const FomodInstaller& installer,
                       const ExpandedAtoms& atoms,
                       const std::vector<std::vector<std::vector<bool>>>& selections,
                       const FomodDependencyContext* context,
                       const InferenceOverrides* overrides)
{
    SimulatedTree tree;
    std::unordered_map<std::string, std::string> flags;

    // Phase 1: Required files
    for (const auto& atom : atoms.required)
    {
        apply_atom(tree, atom);
    }

    // Phase 2: Selected plugin files (normal, non-always) in step order.
    // Also accumulate flags. Auto-select Required-type plugins.
    int flat_idx = 0;
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];

        // Check step visibility
        bool step_visible_result = true;
        if (step.visible)
        {
            if (overrides && si < overrides->step_visible.size())
                step_visible_result = evaluate_condition_inferred(
                    *step.visible, flags, overrides->step_visible[si], context);
            else
                step_visible_result = evaluate_condition(*step.visible, flags, context);
        }
        if (!step_visible_result)
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

                // Also include Required-type plugins even if not explicitly selected
                auto eff_type = evaluate_plugin_type(plugin, flags, context);
                if (eff_type == PluginType::Required)
                    selected = true;

                if (selected)
                {
                    // Accumulate flags
                    for (const auto& [name, value] : plugin.condition_flags)
                        flags[name] = value;

                    // Add normal (non-auto) atoms
                    if (flat_idx < static_cast<int>(atoms.per_plugin.size()))
                    {
                        for (const auto& atom : atoms.per_plugin[flat_idx])
                        {
                            if (!atom.always_install && !atom.install_if_usable)
                                apply_atom(tree, atom);
                        }
                    }
                }
                flat_idx++;
            }
        }
    }

    // Phase 3: Always-install and installIfUsable atoms from ALL plugins
    flat_idx = 0;
    for (const auto& step : installer.steps)
    {
        for (const auto& group : step.groups)
        {
            for (const auto& plugin : group.plugins)
            {
                auto eff_type = evaluate_plugin_type(plugin, flags, context);
                if (flat_idx < static_cast<int>(atoms.per_plugin.size()))
                {
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

    return tree;
}

}  // namespace mo2core
