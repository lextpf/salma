#pragma once

#include "Export.h"
#include "FomodIR.h"

#include <pugixml.hpp>
#include <string>

namespace mo2core
{

/**
 * @class FomodIRParser
 * @brief Transforms a parsed ModuleConfig.xml DOM into the FomodIR structures.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodService
 *
 * Walks a pugixml document and produces a fully populated `FomodInstaller` IR.
 * All paths in the resulting IR are normalized and prefixed with
 * `archive_prefix` so that downstream consumers can map them directly to
 * archive entries without additional path fixup.
 *
 * ## :material-swap-horizontal: Parsing Pipeline
 *
 * 1. `parse` - entry point; extracts required files, steps, and conditional
 *    patterns from the
 * document root.
 * 2. `compile_condition` - recursively converts `<dependencies>` /
 *    `<dependencyType>` nodes
 * into a `FomodCondition` tree.
 * 3. `parse_file_entry` - converts a single `<file>` or `<folder>` element,
 *    applying prefix
 * and destination resolution.
 * 4. `parse_group_type` - maps the XML type string to `FomodGroupType`.
 */
class FomodIRParser
{
public:
    static MO2_API FomodInstaller parse(const pugi::xml_document& doc,
                                        const std::string& archive_prefix);

private:
    static FomodCondition compile_condition(const pugi::xml_node& deps_node);
    static FomodFileEntry parse_file_entry(const pugi::xml_node& node,
                                           const std::string& archive_prefix);
    static FomodGroupType parse_group_type(const std::string& type_str);
};

}  // namespace mo2core
