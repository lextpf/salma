#pragma once

#include "Export.h"
#include "Types.h"

#include <pugixml.hpp>
#include <string>
#include <vector>

namespace mo2core
{

/// Lowercase a string (safe for MSVC plain char via unsigned char cast).
MO2_API std::string to_lower(const std::string& s);

/// Normalize an archive/mod path: lowercase, backslash-to-forward-slash,
/// strip leading "./" and "/", strip trailing "/", collapse "//".
MO2_API std::string normalize_path(const std::string& p);

/// Generate a random hex string of the given length using a thread-local RNG.
MO2_API std::string random_hex_string(size_t length = 12);

/// Respect the FOMOD `order` attribute on installSteps, optionalFileGroups, and plugins.
/// Values: "Ascending" (default, alphabetical by name), "Descending", "Explicit" (document order).
MO2_API std::vector<pugi::xml_node> get_ordered_nodes(const pugi::xml_node& parent,
                                                      const char* child_name);

/// Parse an XML boolean attribute using XML Schema semantics.
/// Returns true for "true"/"1" (case-insensitive), false otherwise.
MO2_API bool xml_bool_attribute_true(const pugi::xml_attribute& attr);

/// Map a FOMOD plugin type name string to its PluginType enum value.
MO2_API PluginType parse_plugin_type_string(const std::string& type_name);

/// FNV-1a 64-bit hash.
MO2_API uint64_t fnv1a_hash(const char* data, size_t size);

}  // namespace mo2core
