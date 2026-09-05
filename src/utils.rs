/*!
 * @brief defines shared string, path, hash, and FOMOD parsing helpers.
 * @author Alex (https://github.com/lextpf)
 *
 * string folding and path normalization are ASCII-only. normalize_path and
 * normalize_destination_for_join use different operation order and are not interchangeable.
 *
 * ### :material-shield-lock: path containment
 *
 * path name screens do not prove containment. callers that join untrusted input must also verify
 * the result with is_inside.
 */

use crate::types::PluginType;
use std::path::{Component, Path, PathBuf};

/**
 * @fn to_lower(&str) -> String
 * @brief fold ASCII letters and leave non-ASCII text unchanged.
 * @author Alex (https://github.com/lextpf)
 *
 * bytes >= 0x80 pass through, and every byte of a multi-byte UTF-8 sequence is >= 0x80, so
 * non-ASCII text is untouched.
 */
pub fn to_lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/**
 * @fn normalize_path(&str) -> String
 * @brief normalize lexically with ASCII-only case folding and no filesystem access.
 * @author Alex (https://github.com/lextpf)
 *
 * applies ASCII lowercase, slash conversion, leading and trailing slash removal, duplicate-slash
 * collapse, then removal of dot segments.
 *
 * @verbatim
 * ./\Textures\\LOD/ -> textures/lod
 * @endverbatim
 *
 * normalize_destination_for_join processes leading markers in a different order. for .//foo it
 * returns /foo, while `normalize_path` returns foo.
 */
pub fn normalize_path(p: &str) -> String {
    let lowered = to_lower(p).replace('\\', "/");
    let mut s: &str = &lowered;
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    while let Some(rest) = s.strip_prefix('/') {
        s = rest;
    }
    let s = s.trim_end_matches('/');
    let mut collapsed = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '/' && collapsed.ends_with('/') {
            continue;
        }
        collapsed.push(c);
    }
    // remove "." and ".." path components to prevent directory traversal.
    let parts: Vec<&str> = collapsed
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != "." && *seg != "..")
        .collect();
    parts.join("/")
}

pub const fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 14695981039346656037;
    let mut i = 0;
    while i < data.len() {
        hash ^= data[i] as u64;
        hash = hash.wrapping_mul(1099511628211);
        i += 1;
    }
    hash
}

/**
 * @fn hash_combine(&mut u64, u64)
 * @brief apply boost-style 64-bit mixing with wrapping arithmetic.
 * @author Alex (https://github.com/lextpf)
 *
 * @code{.text}
 * seed ^= v + 0x9e3779b97f4a7c15 + (seed << 6) + (seed >> 2)
 * @endcode
 *
 * all additions wrap at 64 bits.
 */
pub fn hash_combine(seed: &mut u64, v: u64) {
    *seed ^= v
        .wrapping_add(0x9e3779b97f4a7c15)
        .wrapping_add(*seed << 6)
        .wrapping_add(*seed >> 2);
}

/**
 * @brief define the default scratch-name token length in characters.
 * @author Alex (https://github.com/lextpf)
 */
pub const RANDOM_HEX_DEFAULT_LEN: usize = 12;

/**
 * @fn random_hex_string(usize) -> String
 * @brief produce non-cryptographic thread-local hexadecimal tokens.
 * @author Alex (https://github.com/lextpf)
 *
 * the source is a thread-local SplitMix64 stream, seeded once per thread from std's OS-seeded
 * hasher entropy mixed with the system clock, so no external `rand` dependency is needed.
 */
pub fn random_hex_string(length: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    use std::cell::Cell;
    thread_local! {
        // per-thread state avoids contention when several threads extract concurrently.
        static RNG_STATE: Cell<u64> = Cell::new(random_seed());
    }
    RNG_STATE.with(|state| {
        let mut out = String::with_capacity(length);
        for _ in 0..length {
            // one SplitMix64 step per character.
            let x = state.get().wrapping_add(0x9e3779b97f4a7c15);
            state.set(x);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^= z >> 31;
            // top 4 bits of the mixed output select the hex digit (0..=15).
            out.push(HEX[(z >> 60) as usize] as char);
        }
        out
    })
}

// build a per-thread seed from std's OS-seeded hasher entropy plus the system clock.
fn random_seed() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    // each RandomState carries fresh std-internal random keys; finishing an empty hasher yields a
    // value derived from those keys.
    let a = RandomState::new().build_hasher().finish();
    let b = RandomState::new().build_hasher().finish();
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    a ^ b.rotate_left(32) ^ t
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeOrder {
    /**
     * @brief ascending byte-wise lexicographic order on the name attribute (the schema default).
     * @author Alex (https://github.com/lextpf)
     */
    Ascending,
    /**
     * @brief descending byte-wise lexicographic order on the name attribute.
     * @author Alex (https://github.com/lextpf)
     */
    Descending,
    /**
     * @brief document order (no sort).
     * @author Alex (https://github.com/lextpf)
     */
    Explicit,
}

/**
 * @fn parse_node_order(Option<&str>) -> NodeOrder
 * @brief default missing values to Ascending and unknown values to Explicit.
 * @author Alex (https://github.com/lextpf)
 *
 */
pub fn parse_node_order(order_attr: Option<&str>) -> NodeOrder {
    match order_attr.unwrap_or("Ascending") {
        "Descending" => NodeOrder::Descending,
        "Ascending" => NodeOrder::Ascending,
        _ => NodeOrder::Explicit,
    }
}

/**
 * @fn get_ordered_nodes<T,F>(Option<&str>,Vec<T>,F)->Vec<T> where F:Fn(&T)->&str
 * @brief use unstable byte-order sorting, or preserve input order for Explicit.
 * @author Alex (https://github.com/lextpf)
 *
 */
pub fn get_ordered_nodes<T, F>(order_attr: Option<&str>, mut nodes: Vec<T>, name_of: F) -> Vec<T>
where
    F: Fn(&T) -> &str,
{
    match parse_node_order(order_attr) {
        NodeOrder::Descending => nodes.sort_unstable_by(|a, b| name_of(b).cmp(name_of(a))),
        NodeOrder::Ascending => nodes.sort_unstable_by(|a, b| name_of(a).cmp(name_of(b))),
        NodeOrder::Explicit => {}
    }
    nodes
}

/**
 * @fn xml_bool_attribute_true(Option<&str>) -> bool
 * @brief accept only case-insensitive true and the literal 1.
 * @author Alex (https://github.com/lextpf)
 *
 * pass `None` for a missing attribute and `Some(value)` otherwise; [`crate::fomod_ir_parser`] feeds
 * roxmltree attribute values in.
 */
pub fn xml_bool_attribute_true(attr: Option<&str>) -> bool {
    let Some(raw) = attr else {
        return false;
    };
    let value = to_lower(raw);
    value == "true" || value == "1"
}

/**
 * @fn parse_plugin_type_string(&str) -> PluginType
 * @brief map unknown names to PluginType::Optional.
 * @author Alex (https://github.com/lextpf)
 *
 */
pub fn parse_plugin_type_string(type_name: &str) -> PluginType {
    match type_name {
        "Required" => PluginType::Required,
        "Recommended" => PluginType::Recommended,
        "Optional" => PluginType::Optional,
        "NotUsable" => PluginType::NotUsable,
        "CouldBeUsable" => PluginType::CouldBeUsable,
        // unknown type names install as Optional.
        _ => PluginType::Optional,
    }
}

pub fn plugin_type_to_string(plugin_type: PluginType) -> &'static str {
    match plugin_type {
        PluginType::Required => "Required",
        PluginType::Recommended => "Recommended",
        PluginType::Optional => "Optional",
        PluginType::NotUsable => "NotUsable",
        PluginType::CouldBeUsable => "CouldBeUsable",
    }
}

/**
 * @fn normalize_destination_for_join(&str) -> String
 * @brief strip root markers from a FOMOD destination before normalization.
 * @author Alex (https://github.com/lextpf)
 *
 * removes leading slashes before repeated ./ or .\ prefixes. no final slash pass occurs, so
 * .//foo becomes /foo. normalize_path later removes that slash on the normal parser path.
 *
 * @return the stripped path, which can still start with a slash.
 */
pub fn normalize_destination_for_join(destination: &str) -> String {
    let mut s = destination.trim_start_matches(['\\', '/']);
    while let Some(rest) = s.strip_prefix("./").or_else(|| s.strip_prefix(".\\")) {
        s = rest;
    }
    s.to_string()
}

/**
 * @fn resolve_file_destination(&str, &str, bool) -> String
 * @brief derive empty and directory destinations from the source filename.
 * @author Alex (https://github.com/lextpf)
 *
 * | entry  | destination       | intermediate result          |
 * |--------|-------------------|------------------------------|
 * | file   | empty             | source filename              |
 * | file   | trailing slash    | destination plus filename    |
 * | file   | other             | destination                  |
 * | folder | empty             | empty mod-root path          |
 * | folder | other             | destination                  |
 *
 * normalize_destination_for_join processes the intermediate result.
 *
 * @return the destination prepared for later path normalization.
 */
pub fn resolve_file_destination(source: &str, raw_destination: &str, is_file: bool) -> String {
    let mut destination = raw_destination.to_string();
    if is_file && destination.is_empty() {
        destination = source_filename(source).to_string();
    } else if is_file && destination.ends_with(['/', '\\']) {
        destination.push_str(source_filename(source));
    }
    normalize_destination_for_join(&destination)
}

// filename part of a source path: everything after the last / or \, or the whole string when it
// holds no separator.
fn source_filename(source: &str) -> &str {
    match source.rfind(['/', '\\']) {
        Some(pos) => &source[pos + 1..],
        None => source,
    }
}

/**
 * @fn is_safe_destination(&str) -> bool
 * @brief reject drive prefixes without claiming path containment.
 * @author Alex (https://github.com/lextpf)
 *
 * empty and dot-only inputs are accepted as the mod root. normalization removes leading slashes,
 * so the live rejection is a drive prefix in byte position 1.
 *
 * this is not a containment check. a rooted raw path can pass and then replace the base in
 * Path::join. verify the joined path with is_inside when containment is required.
 *
 * @return true when the string passes this screen.
 */
pub fn is_safe_destination(dest: &str) -> bool {
    if dest.is_empty() {
        return true;
    }
    let norm = normalize_path(dest);
    if norm.is_empty() {
        return true;
    }
    let bytes = norm.as_bytes();
    // normalize_path removes leading slashes, so only the drive-prefix test can reject.
    if bytes[0] == b'/' || (bytes.len() >= 2 && bytes[1] == b':') {
        return false;
    }
    true
}

// the C-locale isspace set: space, \t, \n, \v, \f, \r.
// bytes of 0x80 and above never count, so multi-byte UTF-8 is unaffected.
fn is_c_locale_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/**
 * @fn is_safe_mod_name(&str) -> bool
 * @brief reject path syntax and windows-reserved basenames.
 * @author Alex (https://github.com/lextpf)
 *
 * | rejected form          | rule                                      |
 * |------------------------|-------------------------------------------|
 * | empty or edge space    | C-locale whitespace                       |
 * | path                   | separator or absolute path                |
 * | dot name               | . or ..                                   |
 * | trailing dot           | windows removes it                        |
 * | reserved device stem   | CON, PRN, AUX, NUL, COM1-9, or LPT1-9     |
 *
 * windows drive-relative names such as C:evil pass this screen. verify the joined path with
 * is_inside.
 *
 * @return true when the name passes this screen.
 */
pub fn is_safe_mod_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let bytes = name.as_bytes();
    if is_c_locale_space(bytes[0]) || is_c_locale_space(bytes[bytes.len() - 1]) {
        return false;
    }

    if name.contains('/') || name.contains('\\') {
        return false;
    }
    if Path::new(name).is_absolute() {
        return false;
    }

    if name == "." || name == ".." {
        return false;
    }

    if name.ends_with('.') {
        return false;
    }

    let mut stem = to_lower(name);
    if let Some(dot) = stem.rfind('.') {
        stem.truncate(dot);
    }
    !matches!(
        stem.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

// normalize a path lexically, without touching the filesystem: drop . components, fold name/..
// pairs, drop .. directly after a root directory, keep a leading .. in a relative path, and turn an
// all-elided non-empty input into ..
// an empty input stays empty.
fn lexically_normal(p: &Path) -> PathBuf {
    if p.as_os_str().is_empty() {
        return PathBuf::new();
    }
    let mut out = PathBuf::new();
    let mut normals = 0usize;
    let mut has_root = false;
    for comp in p.components() {
        match comp {
            Component::Prefix(_) => out.push(comp.as_os_str()),
            Component::RootDir => {
                out.push(comp.as_os_str());
                has_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if normals > 0 {
                    out.pop();
                    normals -= 1;
                } else if !has_root {
                    // leading ".." in a relative path is preserved; ".." directly after a root
                    // directory is dropped.
                    out.push("..");
                }
            }
            Component::Normal(s) => {
                out.push(s);
                normals += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

// canonicalize a path that does not have to exist.
// `std::fs::canonicalize` fails on a nonexistent path.
fn weakly_canonical(p: &Path) -> std::io::Result<PathBuf> {
    use std::io::ErrorKind;
    match std::fs::canonicalize(p) {
        Ok(canon) => return Ok(canon),
        Err(e) if e.kind() != ErrorKind::NotFound && e.kind() != ErrorKind::NotADirectory => {
            return Err(e);
        }
        Err(_) => {}
    }
    let comps: Vec<Component> = p.components().collect();
    // try progressively shorter leading prefixes until one canonicalizes.
    for split in (1..comps.len()).rev() {
        let head: PathBuf = comps[..split].iter().collect();
        match std::fs::canonicalize(&head) {
            Ok(canon) => {
                let mut out = canon;
                for comp in &comps[split..] {
                    out.push(comp.as_os_str());
                }
                return Ok(lexically_normal(&out));
            }
            Err(e) if e.kind() != ErrorKind::NotFound && e.kind() != ErrorKind::NotADirectory => {
                return Err(e);
            }
            Err(_) => {}
        }
    }
    // nothing exists: purely lexical result.
    Ok(lexically_normal(p))
}

/**
 * @fn is_inside(&Path, &Path) -> bool
 * @brief resolve existing symlinks and fail closed on filesystem errors.
 * @author Alex (https://github.com/lextpf)
 *
 * the check performs blocking filesystem access, resolves existing symlinks, and fails closed.
 * nonexistent suffixes use lexical normalization. equality counts as containment.
 *
 * the result is point-in-time and does not prevent a later symlink race.
 *
 * @return true when the resolved child has the resolved parent as a component prefix.
 */
pub fn is_inside(parent: &Path, child: &Path) -> bool {
    let Ok(canonical_child) = weakly_canonical(child) else {
        return false;
    };
    let Ok(canonical_parent) = weakly_canonical(parent) else {
        return false;
    };
    canonical_child.starts_with(&canonical_parent)
}

/**
 * @fn executable_directory() -> PathBuf
 * @brief return the host executable directory, not the calling module directory.
 * @author Alex (https://github.com/lextpf)
 *
 * do not use it for resources that should follow the calling binary inside MO2: there the host EXE
 * is `ModOrganizer.exe`, so this points at MO2's install root.
 */
pub fn executable_directory() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(dir) = win::module_path_for(std::ptr::null_mut()) {
            return dir;
        }
    }
    current_dir_fallback()
}

/**
 * @fn module_directory(*const core::ffi::c_void) -> PathBuf
 * @brief fall back to the current working directory when module lookup fails.
 * @author Alex (https://github.com/lextpf)
 *
 * falls back to the current working directory when the lookup fails or the platform is not windows.
 */
pub fn module_directory(anchor: *const core::ffi::c_void) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(dir) = win::module_directory_for_address(anchor) {
            return dir;
        }
    }
    #[cfg(not(windows))]
    {
        let _ = anchor;
    }
    current_dir_fallback()
}

// shared fallback for the directory lookups: the current working directory, or "." when even that
// cannot be read.
// never panics.
fn current_dir_fallback() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(windows)]
mod win {
    /*!
     * @brief implements Win32 module path lookup.
     * @author Alex (https://github.com/lextpf)
     *
     * executable_directory and module_directory expose these lookups to the crate.
     */

    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{HMODULE, MAX_PATH};
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };

    // resolve the parent directory of the file backing hmod.
    // pass null to query the host executable.
    pub(super) fn module_path_for(hmod: HMODULE) -> Option<PathBuf> {
        const MAX_RETRIES: u32 = 5;
        let mut buf: Vec<u16> = vec![0; MAX_PATH as usize];
        // safety: buf is a valid, writable u16 buffer of the length passed.
        let mut len = unsafe { GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32) };
        let mut retries = 0;
        while len as usize >= buf.len() && retries < MAX_RETRIES {
            buf.resize(buf.len() * 2, 0);
            // safety: buf was just resized; pointer and length stay in sync.
            len = unsafe { GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32) };
            retries += 1;
        }
        if len > 0 && retries < MAX_RETRIES {
            buf.truncate(len as usize);
            let full = PathBuf::from(OsString::from_wide(&buf));
            return full.parent().map(PathBuf::from);
        }
        None
    }

    // GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, anchor, ...), then module_path_for.
    // `None` if the handle lookup fails.
    pub(super) fn module_directory_for_address(
        anchor: *const core::ffi::c_void,
    ) -> Option<PathBuf> {
        let mut hmod: HMODULE = std::ptr::null_mut();
        // safety: anchor is only inspected as an address (FROM_ADDRESS flag);
        // hmod is a valid out-pointer. UNCHANGED_REFCOUNT means no reference
        // is leaked.
        let ok = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                anchor.cast::<u16>(),
                &mut hmod,
            )
        };
        if ok != 0 {
            return module_path_for(hmod);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_lower_lowercases_upper_case() {
        assert_eq!(to_lower("HELLO"), "hello");
    }

    #[test]
    fn to_lower_preserves_already_lower_case() {
        assert_eq!(to_lower("hello"), "hello");
    }

    #[test]
    fn to_lower_mixed_case() {
        assert_eq!(to_lower("HeLLo WoRLd"), "hello world");
    }

    #[test]
    fn to_lower_empty_string() {
        assert_eq!(to_lower(""), "");
    }

    #[test]
    fn to_lower_non_alpha() {
        assert_eq!(to_lower("123!@#"), "123!@#");
    }

    #[test]
    fn normalize_path_backslash_to_forward_slash() {
        assert_eq!(
            normalize_path("textures\\lod\\file.dds"),
            "textures/lod/file.dds"
        );
    }

    #[test]
    fn normalize_path_strips_leading_dot_slash() {
        assert_eq!(
            normalize_path("./textures/lod/file.dds"),
            "textures/lod/file.dds"
        );
    }

    #[test]
    fn normalize_path_strips_leading_slash() {
        assert_eq!(
            normalize_path("/textures/lod/file.dds"),
            "textures/lod/file.dds"
        );
    }

    #[test]
    fn normalize_path_strips_trailing_slash() {
        assert_eq!(normalize_path("textures/lod/"), "textures/lod");
    }

    #[test]
    fn normalize_path_collapses_double_slash() {
        assert_eq!(
            normalize_path("textures//lod//file.dds"),
            "textures/lod/file.dds"
        );
    }

    #[test]
    fn normalize_path_lowercases_path() {
        assert_eq!(
            normalize_path("Textures\\LOD\\File.DDS"),
            "textures/lod/file.dds"
        );
    }

    #[test]
    fn normalize_path_empty_string() {
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn normalize_path_multiple_dot_slash_prefixes() {
        assert_eq!(normalize_path("././textures/file.dds"), "textures/file.dds");
    }

    #[test]
    fn normalize_path_only_slashes() {
        assert_eq!(normalize_path("///"), "");
    }

    #[test]
    fn normalize_path_mixed_separators_and_prefixes() {
        assert_eq!(normalize_path("./\\Textures\\\\LOD/"), "textures/lod");
    }

    #[test]
    fn random_hex_string_default_length() {
        let s = random_hex_string(RANDOM_HEX_DEFAULT_LEN);
        assert_eq!(s.len(), 12);
    }

    #[test]
    fn random_hex_string_custom_length() {
        let s = random_hex_string(32);
        assert_eq!(s.len(), 32);
    }

    #[test]
    fn random_hex_string_zero_length() {
        let s = random_hex_string(0);
        assert!(s.is_empty());
    }

    #[test]
    fn random_hex_string_only_hex_chars() {
        let s = random_hex_string(100);
        for c in s.chars() {
            assert!(
                c.is_ascii_digit() || ('a'..='f').contains(&c),
                "Non-hex char: {c}"
            );
        }
    }

    #[test]
    fn random_hex_string_two_calls_produce_different_results() {
        let a = random_hex_string(32);
        let b = random_hex_string(32);
        assert_ne!(a, b);
    }

    fn names<'a>(nodes: &'a [(&str, usize)]) -> Vec<&'a str> {
        nodes.iter().map(|n| n.0).collect()
    }

    #[test]
    fn get_ordered_nodes_ascending_order() {
        let nodes = vec![("Charlie", 0), ("Alice", 1), ("Bob", 2)];
        let ordered = get_ordered_nodes(Some("Ascending"), nodes, |n| n.0);
        assert_eq!(names(&ordered), ["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn get_ordered_nodes_descending_order() {
        let nodes = vec![("Alice", 0), ("Charlie", 1), ("Bob", 2)];
        let ordered = get_ordered_nodes(Some("Descending"), nodes, |n| n.0);
        assert_eq!(names(&ordered), ["Charlie", "Bob", "Alice"]);
    }

    #[test]
    fn get_ordered_nodes_explicit_order() {
        let nodes = vec![("Charlie", 0), ("Alice", 1), ("Bob", 2)];
        let ordered = get_ordered_nodes(Some("Explicit"), nodes, |n| n.0);
        assert_eq!(names(&ordered), ["Charlie", "Alice", "Bob"]);
    }

    #[test]
    fn get_ordered_nodes_default_is_ascending() {
        let nodes = vec![("Charlie", 0), ("Alice", 1), ("Bob", 2)];
        let ordered = get_ordered_nodes(None, nodes, |n| n.0);
        assert_eq!(ordered[0].0, "Alice");
    }

    #[test]
    fn get_ordered_nodes_empty_parent() {
        let nodes: Vec<(&str, usize)> = Vec::new();
        let ordered = get_ordered_nodes(Some("Ascending"), nodes, |n| n.0);
        assert!(ordered.is_empty());
    }

    #[test]
    fn xml_bool_attribute_true_string() {
        assert!(xml_bool_attribute_true(Some("true")));
    }

    #[test]
    fn xml_bool_attribute_true_upper_case() {
        assert!(xml_bool_attribute_true(Some("TRUE")));
    }

    #[test]
    fn xml_bool_attribute_one() {
        assert!(xml_bool_attribute_true(Some("1")));
    }

    #[test]
    fn xml_bool_attribute_false_string() {
        assert!(!xml_bool_attribute_true(Some("false")));
    }

    #[test]
    fn xml_bool_attribute_zero() {
        assert!(!xml_bool_attribute_true(Some("0")));
    }

    #[test]
    fn xml_bool_attribute_missing_attribute() {
        assert!(!xml_bool_attribute_true(None));
    }

    #[test]
    fn parse_plugin_type_string_required() {
        assert_eq!(parse_plugin_type_string("Required"), PluginType::Required);
    }

    #[test]
    fn parse_plugin_type_string_recommended() {
        assert_eq!(
            parse_plugin_type_string("Recommended"),
            PluginType::Recommended
        );
    }

    #[test]
    fn parse_plugin_type_string_optional() {
        assert_eq!(parse_plugin_type_string("Optional"), PluginType::Optional);
    }

    #[test]
    fn parse_plugin_type_string_not_usable() {
        assert_eq!(parse_plugin_type_string("NotUsable"), PluginType::NotUsable);
    }

    #[test]
    fn parse_plugin_type_string_could_be_usable() {
        assert_eq!(
            parse_plugin_type_string("CouldBeUsable"),
            PluginType::CouldBeUsable
        );
    }

    #[test]
    fn parse_plugin_type_string_unknown_defaults_to_optional() {
        assert_eq!(parse_plugin_type_string("Bogus"), PluginType::Optional);
    }

    #[test]
    fn parse_plugin_type_string_empty_defaults_to_optional() {
        assert_eq!(parse_plugin_type_string(""), PluginType::Optional);
    }

    #[test]
    fn is_safe_mod_name_rejects_traversal_forward() {
        assert!(!is_safe_mod_name("../outside"));
    }

    #[test]
    fn is_safe_mod_name_rejects_traversal_backward() {
        assert!(!is_safe_mod_name("..\\outside"));
    }

    #[test]
    fn is_safe_mod_name_rejects_bare_double_dot() {
        assert!(!is_safe_mod_name(".."));
    }

    #[test]
    fn is_safe_mod_name_rejects_absolute_windows_path() {
        assert!(!is_safe_mod_name("C:\\temp\\evil"));
    }

    #[test]
    fn is_safe_mod_name_rejects_drive_root() {
        assert!(!is_safe_mod_name("C:\\"));
    }

    #[test]
    fn is_safe_mod_name_rejects_leading_slash() {
        assert!(!is_safe_mod_name("/etc/passwd"));
    }

    #[test]
    fn is_safe_mod_name_rejects_forward_slash_inside() {
        assert!(!is_safe_mod_name("foo/bar"));
    }

    #[test]
    fn is_safe_mod_name_rejects_backslash_inside() {
        assert!(!is_safe_mod_name("foo\\bar"));
    }

    #[test]
    fn is_safe_mod_name_rejects_empty() {
        assert!(!is_safe_mod_name(""));
    }

    #[test]
    fn is_safe_mod_name_rejects_whitespace_only() {
        assert!(!is_safe_mod_name("   "));
    }

    #[test]
    fn is_safe_mod_name_rejects_bare_dot() {
        assert!(!is_safe_mod_name("."));
    }

    #[test]
    fn is_safe_mod_name_rejects_trailing_dot() {
        assert!(!is_safe_mod_name("MyMod."));
    }

    #[test]
    fn is_safe_mod_name_rejects_trailing_space() {
        assert!(!is_safe_mod_name("MyMod "));
    }

    #[test]
    fn is_safe_mod_name_rejects_leading_space() {
        assert!(!is_safe_mod_name(" MyMod"));
    }

    #[test]
    fn is_safe_mod_name_rejects_windows_reserved_con() {
        assert!(!is_safe_mod_name("CON"));
        assert!(!is_safe_mod_name("con"));
        assert!(!is_safe_mod_name("CON.txt"));
    }

    #[test]
    fn is_safe_mod_name_rejects_windows_reserved_com_lpt() {
        assert!(!is_safe_mod_name("COM1"));
        assert!(!is_safe_mod_name("LPT9"));
        assert!(!is_safe_mod_name("nul"));
    }

    #[test]
    fn is_safe_mod_name_accepts_simple_name() {
        assert!(is_safe_mod_name("SkyUI"));
    }

    #[test]
    fn is_safe_mod_name_accepts_name_with_spaces() {
        assert!(is_safe_mod_name("My Mod 1.2"));
    }

    #[test]
    fn is_safe_mod_name_accepts_dots_inside() {
        assert!(is_safe_mod_name("A.B.C"));
    }

    #[test]
    fn is_safe_mod_name_accepts_hyphens_and_underscores() {
        assert!(is_safe_mod_name("My_Cool-Mod"));
    }

    // containment invariant: even when a hostile name slips past is_safe_mod_name, the is_inside
    // check on the joined path catches it. this exercises the pairing the upload controller relies
    // on. the temp directory name is unique to this test so concurrent suites cannot collide.
    #[test]
    fn is_inside_rejects_mod_name_traversal_generated_path_stays_inside_mods_dir() {
        let tmp = std::env::temp_dir().join("salma_rs_modname_containment_test");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        assert!(is_inside(&tmp, &tmp.join("SkyUI")));
        assert!(!is_inside(&tmp, &tmp.join("..").join("escape")));
        std::fs::remove_dir_all(&tmp).expect("remove temp dir");
    }

    #[test]
    fn fnv1a_hash_known_vectors() {
        assert_eq!(fnv1a_hash(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a_hash(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a_hash(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn fnv1a_hash_const_eval() {
        const H: u64 = fnv1a_hash(b"flagDependency");
        assert_eq!(H, fnv1a_hash(b"flagDependency"));
    }

    #[test]
    fn fnv1a_hash_matches_reference_vectors() {
        let cases: &[(&[u8], u64)] = &[
            (b"", 0xcbf29ce484222325),
            (b"a", 0xaf63dc4c8601ec8c),
            (b"salma", 0xbacb1f0d8e9d1005),
            (
                b"The quick brown fox jumps over the lazy dog",
                0xf3f9b7f5e7e47110,
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                fnv1a_hash(input),
                *expected,
                "fnv1a mismatch for {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn hash_combine_matches_cpp_formula() {
        let mut seed = 0u64;
        hash_combine(&mut seed, 0);
        assert_eq!(seed, 0x9e3779b97f4a7c15);

        let mut seed = 14695981039346656037u64;
        hash_combine(&mut seed, 42);
        assert_eq!(seed, 0x0629c6f72cf9ed6d);

        let mut seed = 0u64;
        hash_combine(&mut seed, fnv1a_hash(b"flag"));
        hash_combine(&mut seed, fnv1a_hash(b"value"));
        assert_eq!(seed, 0x3693dc4ec01f72e6);
    }

    #[test]
    fn plugin_type_to_string_round_trips() {
        for t in [
            PluginType::Required,
            PluginType::Recommended,
            PluginType::Optional,
            PluginType::NotUsable,
            PluginType::CouldBeUsable,
        ] {
            assert_eq!(parse_plugin_type_string(plugin_type_to_string(t)), t);
        }
    }

    #[test]
    fn normalize_destination_for_join_strips_root_markers() {
        assert_eq!(normalize_destination_for_join("/"), "");
        assert_eq!(normalize_destination_for_join("\\"), "");
        assert_eq!(normalize_destination_for_join("//dir/file"), "dir/file");
        assert_eq!(normalize_destination_for_join("./dir"), "dir");
        assert_eq!(normalize_destination_for_join(".\\dir"), "dir");
        assert_eq!(normalize_destination_for_join("././dir"), "dir");
        assert_eq!(normalize_destination_for_join("dir/file"), "dir/file");
    }

    #[test]
    fn normalize_destination_for_join_sequential_strip_quirk() {
        assert_eq!(normalize_destination_for_join(".//foo"), "/foo");
    }

    #[test]
    fn resolve_file_destination_file_rules() {
        assert_eq!(
            resolve_file_destination("a/b/plugin.esp", "", true),
            "plugin.esp"
        );
        assert_eq!(
            resolve_file_destination("plugin.esp", "", true),
            "plugin.esp"
        );
        assert_eq!(
            resolve_file_destination("a\\b\\plugin.esp", "dest/", true),
            "dest/plugin.esp"
        );
        assert_eq!(
            resolve_file_destination("a/plugin.esp", "dest\\", true),
            "dest\\plugin.esp"
        );
        assert_eq!(
            resolve_file_destination("a/plugin.esp", "/dest.esp", true),
            "dest.esp"
        );
    }

    #[test]
    fn resolve_file_destination_folder_rules() {
        assert_eq!(resolve_file_destination("some/folder", "", false), "");
        assert_eq!(
            resolve_file_destination("some/folder", "dest/", false),
            "dest/"
        );
        assert_eq!(
            resolve_file_destination("some/folder", "/dest", false),
            "dest"
        );
    }

    #[test]
    fn is_safe_destination_rules() {
        assert!(is_safe_destination(""));
        assert!(is_safe_destination("textures/file.dds"));
        // traversal-only inputs normalize to empty, which resolves to the mod root and is safe
        // (normalize_path already dropped the "..").
        assert!(is_safe_destination(".."));
        assert!(is_safe_destination("../.."));
        assert!(is_safe_destination("a/../b"));
        assert!(!is_safe_destination("C:/evil"));
        assert!(!is_safe_destination("c:\\evil"));
        // the consequence is pinned by fomod_service's
        // enqueue_entry_reproduces_the_rooted_destination_hole.
    }

    #[test]
    fn is_inside_accepts_equal_paths_like_cpp_dot_relative() {
        let tmp = std::env::temp_dir();
        assert!(is_inside(&tmp, &tmp));
    }

    #[test]
    fn is_inside_rejects_sibling_with_common_prefix() {
        let tmp = std::env::temp_dir().join("salma_rs_sibling_prefix_test");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let parent = tmp.join("foo");
        let sibling = tmp.join("foobar");
        assert!(!is_inside(&parent, &sibling));
        std::fs::remove_dir_all(&tmp).expect("remove temp dir");
    }

    #[test]
    fn lexically_normal_matches_cpp_rules() {
        assert_eq!(lexically_normal(Path::new("a/b/../c")), Path::new("a/c"));
        assert_eq!(lexically_normal(Path::new("a/./b")), Path::new("a/b"));
        assert_eq!(lexically_normal(Path::new("a/..")), Path::new("."));
        assert_eq!(lexically_normal(Path::new("../a")), Path::new("../a"));
        assert_eq!(lexically_normal(Path::new("")), Path::new(""));
    }

    #[test]
    fn weakly_canonical_handles_nonexistent_tail() {
        let tmp = std::env::temp_dir();
        let child = tmp.join("salma_rs_wc_nonexistent").join("sub");
        let wc = weakly_canonical(&child).expect("weakly_canonical");
        assert!(wc.ends_with(Path::new("salma_rs_wc_nonexistent/sub")));
        let lex = weakly_canonical(Path::new("salma_rs_no_such_dir/x/../y")).expect("lexical");
        assert_eq!(lex, Path::new("salma_rs_no_such_dir/y"));
    }

    #[test]
    fn executable_directory_exists() {
        let dir = executable_directory();
        assert!(dir.is_dir(), "not a directory: {}", dir.display());
    }

    #[test]
    fn module_directory_of_local_symbol_is_executable_directory() {
        let anchor = module_directory as *const core::ffi::c_void;
        assert_eq!(module_directory(anchor), executable_directory());
    }
}
