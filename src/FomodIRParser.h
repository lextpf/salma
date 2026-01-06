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
    /**
     * @brief Parse a ModuleConfig.xml DOM into a fully populated FomodInstaller IR.
     *
     * Walks the pugixml document tree, extracting module dependencies, required
     * files, ordered install steps (with groups, plugins, type descriptors, and
     * condition flags), and conditional file install patterns. All source paths
     * in the resulting IR are prefixed with @p archive_prefix and normalized.
     *
     * @param doc        Parsed pugixml document of a FOMOD ModuleConfig.xml.
     * @param archive_prefix  Path prefix prepended to every `source` attribute
     *                        (e.g. the subdirectory within the archive that
     *                        contains the mod files). May be empty.
     * @return A fully populated FomodInstaller. Returns a default-constructed
     *         (empty) installer if the document has no `<config>` root element.
     * @pre @p doc must be a successfully parsed pugixml document (not empty/failed).
     * @throw Does not throw. Malformed or unknown XML elements are logged and skipped.
     *        Excessively deep condition trees are truncated to prevent stack overflow.
     */
    static MO2_API FomodInstaller parse(const pugi::xml_document& doc,
                                        const std::string& archive_prefix);

private:
    /** @brief Recursively convert a `<dependencies>` XML node into a FomodCondition tree. */
    static FomodCondition compile_condition(const pugi::xml_node& deps_node);

    /** @brief Convert a single `<file>` or `<folder>` element into a FomodFileEntry. */
    static FomodFileEntry parse_file_entry(const pugi::xml_node& node,
                                           const std::string& archive_prefix);

    /** @brief Map a group type string (e.g. "SelectExactlyOne") to the FomodGroupType enum. */
    static FomodGroupType parse_group_type(const std::string& type_str);
};

}  // namespace mo2core
