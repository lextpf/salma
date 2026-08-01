//! Shared utility functions - Rust port of `src/Utils.hpp` / `src/Utils.cpp`.
//!
//! Every function here mirrors the C++ implementation byte-for-byte in its
//! observable behavior; see `PARITY-NOTES.md` ("Task 3 - Utils port")
//! for the mapping decisions, notably:
//!
//! - The C++ compile-time dispatch helpers (`EnumStringMap`, `HashDispatch`,
//!   `operator""_h`, `no_hash_collisions`) are replaced by plain `match`
//!   expressions ([`parse_plugin_type_string`], [`plugin_type_to_string`]).
//! - The pugixml-typed helpers (`get_ordered_nodes`,
//!   `xml_bool_attribute_true`) are ported generically over strings; Task 4
//!   wires them to the real XML crate.

use crate::types::PluginType;
use std::path::{Component, Path, PathBuf};

/// Lowercase a string at the byte level.
///
/// Mirror of `mo2core::to_lower`: the C++ implementation casts each byte to
/// `unsigned char` and passes it to `std::tolower` under the default "C"
/// locale, which lowercases only ASCII `A`-`Z` and leaves bytes >= 0x80
/// unchanged. `str::to_ascii_lowercase` has exactly those semantics (and
/// multi-byte UTF-8 sequences consist solely of bytes >= 0x80, so they pass
/// through untouched). This is a byte-level lowercaser, not a Unicode
/// case-folder.
pub fn to_lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Normalize an archive/mod path. Mirror of `mo2core::normalize_path`.
///
/// The pipeline is applied in the same order as the C++ implementation:
///
/// 1. lowercase ([`to_lower`])
/// 2. backslash to forward slash
/// 3. strip leading `./` prefixes, then strip leading `/` prefixes
/// 4. strip trailing `/`
/// 5. single-pass collapse of consecutive slashes
/// 6. drop `.` and `..` segments (syntactic strip, no filesystem resolution)
///
/// The output is lowercase, uses forward slashes only, has no leading or
/// trailing slashes, no repeated `/`, and no `.` or `..` path segments.
pub fn normalize_path(p: &str) -> String {
    let lowered = to_lower(p).replace('\\', "/");
    // Strip leading "./" or "/" prefixes that some archivers emit. Two
    // sequential loops, exactly as in the C++ implementation.
    let mut s: &str = &lowered;
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    while let Some(rest) = s.strip_prefix('/') {
        s = rest;
    }
    // Strip trailing "/".
    let s = s.trim_end_matches('/');
    // Single-pass collapse of consecutive slashes.
    let mut collapsed = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '/' && collapsed.ends_with('/') {
            continue;
        }
        collapsed.push(c);
    }
    // Remove "." and ".." path components to prevent directory traversal.
    let parts: Vec<&str> = collapsed
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != "." && *seg != "..")
        .collect();
    parts.join("/")
}

/// FNV-1a 64-bit hash. Mirror of `mo2core::fnv1a_hash` in `src/Utils.hpp`.
///
/// `h0 = 0xCBF29CE484222325`; `h_{i+1} = (h_i XOR b_i) * 0x100000001B3`,
/// with u64 wrapping multiplication, fed byte-stream order. `const fn` so it
/// can seed compile-time constants, mirroring the C++ `constexpr`.
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

/// Boost-style hash combiner. Mirror of `hash_combine` in
/// `src/FomodCSPPrecompute.cpp` (it lives in the CSP module in C++, not in
/// Utils; hosted here so Task 8 can consume it):
///
/// `seed ^= v + 0x9e3779b97f4a7c15 + (seed << 6) + (seed >> 2)`
///
/// with u64 wrapping addition throughout.
pub fn hash_combine(seed: &mut u64, v: u64) {
    *seed ^= v
        .wrapping_add(0x9e3779b97f4a7c15)
        .wrapping_add(*seed << 6)
        .wrapping_add(*seed >> 2);
}

/// The C++ `random_hex_string` default argument (`length = 12`). Rust has no
/// default arguments, so callers spell `random_hex_string(RANDOM_HEX_DEFAULT_LEN)`.
pub const RANDOM_HEX_DEFAULT_LEN: usize = 12;

/// Generate a random lowercase-hex string of exactly `length` characters
/// using a thread-local RNG. Mirror of `mo2core::random_hex_string`.
///
/// Output alphabet (`0-9a-f`) and length semantics are identical to C++.
/// The randomness source differs: C++ uses a thread-local `std::mt19937`
/// seeded from `std::random_device`; this port uses a thread-local SplitMix64
/// stream seeded from `RandomState` (OS-seeded std entropy) mixed with the
/// system clock, avoiding an external `rand` dependency. Both are
/// non-cryptographic; every C++ call site uses the value as a scratch-name /
/// uniqueness token.
pub fn random_hex_string(length: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    use std::cell::Cell;
    thread_local! {
        // thread_local avoids contention when multiple threads extract
        // concurrently (same rationale as the C++ thread_local mt19937).
        static RNG_STATE: Cell<u64> = Cell::new(random_seed());
    }
    RNG_STATE.with(|state| {
        let mut out = String::with_capacity(length);
        for _ in 0..length {
            // One SplitMix64 step per character (the C++ draws one
            // uniform_int_distribution value per character).
            let x = state.get().wrapping_add(0x9e3779b97f4a7c15);
            state.set(x);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^= z >> 31;
            // Top 4 bits of the mixed output select the hex digit (0..=15).
            out.push(HEX[(z >> 60) as usize] as char);
        }
        out
    })
}

/// Build a per-thread seed from std's OS-seeded hasher entropy plus the
/// system clock. Plays the role of the C++ `std::random_device{}()` seeding.
fn random_seed() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    // Each RandomState carries fresh std-internal random keys; finishing an
    // empty hasher yields a value derived from those keys.
    let a = RandomState::new().build_hasher().finish();
    let b = RandomState::new().build_hasher().finish();
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    a ^ b.rotate_left(32) ^ t
}

/// Node ordering mode for the FOMOD `order` attribute on `installSteps`,
/// `optionalFileGroups`, and `plugins` collections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeOrder {
    /// Alphabetical by `name` attribute (the schema default).
    Ascending,
    /// Reverse alphabetical by `name` attribute.
    Descending,
    /// Document order (no sort).
    Explicit,
}

/// Map an `order` attribute value to a [`NodeOrder`], mirroring the exact
/// C++ branch structure in `mo2core::get_ordered_nodes`:
///
/// - missing attribute defaults to `"Ascending"`
/// - `"Descending"` sorts descending, `"Ascending"` sorts ascending
/// - anything else (including `"Explicit"`, unknown values, and any casing
///   mismatch) falls through to document order
pub fn parse_node_order(order_attr: Option<&str>) -> NodeOrder {
    match order_attr.unwrap_or("Ascending") {
        "Descending" => NodeOrder::Descending,
        "Ascending" => NodeOrder::Ascending,
        _ => NodeOrder::Explicit,
    }
}

/// Respect the FOMOD `order` attribute. Generic port of
/// `mo2core::get_ordered_nodes`: the C++ takes a pugixml parent node, reads
/// its `order` attribute, and sorts the named children by their `name`
/// attribute; this port takes the already-read attribute value plus the
/// collected children (in document order) and a name projection. Task 4 wires
/// it to the real XML crate.
///
/// Sorting uses `sort_unstable_by`, matching `std::ranges::sort`: when two
/// nodes share the same name under Ascending/Descending, their relative order
/// is unspecified in both implementations.
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

/// Parse an XML boolean attribute using XML Schema semantics. Generic port of
/// `mo2core::xml_bool_attribute_true`: the C++ takes a pugixml attribute; this
/// port takes `None` for a missing attribute and `Some(value)` otherwise.
///
/// Returns true for `"true"`/`"1"` (case-insensitive), false otherwise
/// (including missing).
pub fn xml_bool_attribute_true(attr: Option<&str>) -> bool {
    let Some(raw) = attr else {
        return false;
    };
    let value = to_lower(raw);
    value == "true" || value == "1"
}

/// Map a FOMOD plugin type name string to its [`PluginType`] value. Mirror of
/// `mo2core::parse_plugin_type_string` (an `EnumStringMap` lookup in C++,
/// a plain `match` here): unrecognized names, including the empty string,
/// default to [`PluginType::Optional`].
pub fn parse_plugin_type_string(type_name: &str) -> PluginType {
    match type_name {
        "Required" => PluginType::Required,
        "Recommended" => PluginType::Recommended,
        "Optional" => PluginType::Optional,
        "NotUsable" => PluginType::NotUsable,
        "CouldBeUsable" => PluginType::CouldBeUsable,
        // C++ EnumStringMap default_value on lookup miss.
        _ => PluginType::Optional,
    }
}

/// Map a [`PluginType`] value to its FOMOD type name string. Mirror of
/// `mo2core::plugin_type_to_string`. The C++ `EnumStringMap` returns
/// `"Unknown"` on a lookup miss, but every `PluginType` value is in the map,
/// so the miss is unreachable; the exhaustive `match` here encodes that
/// directly.
pub fn plugin_type_to_string(plugin_type: PluginType) -> &'static str {
    match plugin_type {
        PluginType::Required => "Required",
        PluginType::Recommended => "Recommended",
        PluginType::Optional => "Optional",
        PluginType::NotUsable => "NotUsable",
        PluginType::CouldBeUsable => "CouldBeUsable",
    }
}

/// Strip leading slashes and `./` from FOMOD destinations so they are safe to
/// join with a mod-root directory path. Mirror of
/// `mo2core::normalize_destination_for_join`.
///
/// FOMOD destinations are mod-root-relative: values like `\` or `/` mean
/// "root", not an absolute filesystem path. As in C++, the two strip loops
/// run sequentially (all leading slashes first, then `./` / `.\` prefixes),
/// so an input like `.//foo` keeps the slash the `./` strip re-exposes.
pub fn normalize_destination_for_join(destination: &str) -> String {
    let mut s = destination.trim_start_matches(['\\', '/']);
    while let Some(rest) = s.strip_prefix("./").or_else(|| s.strip_prefix(".\\")) {
        s = rest;
    }
    s.to_string()
}

/// Resolve a `<file>`/`<folder>` node's destination, handling empty
/// destinations and trailing-slash directory semantics, then normalize for
/// filesystem join. Mirror of `mo2core::resolve_file_destination`:
///
/// - `<file>` with an empty destination installs to the source's filename
/// - `<file>` with a destination ending in `/` or `\` treats the destination
///   as a directory and appends the source's filename
/// - `<folder>` destinations pass through unchanged (empty stays empty)
/// - the result goes through [`normalize_destination_for_join`]
pub fn resolve_file_destination(source: &str, raw_destination: &str, is_file: bool) -> String {
    let mut destination = raw_destination.to_string();
    if is_file && destination.is_empty() {
        destination = source_filename(source).to_string();
    } else if is_file && destination.ends_with(['/', '\\']) {
        destination.push_str(source_filename(source));
    }
    normalize_destination_for_join(&destination)
}

/// Filename part of a source path: everything after the last `/` or `\`, or
/// the whole string when no separator is present (the C++
/// `find_last_of("/\\")` idiom).
fn source_filename(source: &str) -> &str {
    match source.rfind(['/', '\\']) {
        Some(pos) => &source[pos + 1..],
        None => source,
    }
}

/// Reject destination paths that would escape the mod directory via traversal
/// or absolute paths. Mirror of `mo2core::is_safe_destination`.
///
/// [`normalize_path`] strips every `.` and `..` segment, so traversal
/// sequences cannot survive into the normalized form. A non-empty input that
/// consists solely of those segments normalizes to empty, which is also safe
/// (it resolves to the mod root). The remaining guard rejects absolute paths
/// (`/etc/passwd`) and Windows drive letters (`C:/...`).
pub fn is_safe_destination(dest: &str) -> bool {
    if dest.is_empty() {
        return true;
    }
    let norm = normalize_path(dest);
    if norm.is_empty() {
        return true;
    }
    let bytes = norm.as_bytes();
    if bytes[0] == b'/' || (bytes.len() >= 2 && bytes[1] == b':') {
        return false;
    }
    true
}

/// C-locale `isspace` set: space, `\t`, `\n`, `\v`, `\f`, `\r`. Bytes >= 0x80
/// are not whitespace in the "C" locale, so multi-byte UTF-8 is unaffected.
/// (`u8::is_ascii_whitespace` omits `\v`, so it is not a faithful mirror.)
fn is_c_locale_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Reject mod-name strings that are unsafe to use as a single directory
/// component under a mods root. Mirror of `mo2core::is_safe_mod_name`,
/// including the rule order:
///
/// - empty
/// - leading or trailing C-locale whitespace
/// - contains `/` or `\` (path separators)
/// - parses as an absolute path (defensive; unreachable once separators are
///   rejected, kept for parity)
/// - equals `.` or `..`
/// - trailing `.` (CreateFile strips it silently on Windows)
/// - lowercase stem (before the final `.`) matches a Windows reserved device
///   name: CON, PRN, AUX, NUL, COM1-9, LPT1-9
pub fn is_safe_mod_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    // Reject leading/trailing whitespace. Windows trims trailing whitespace
    // in CreateFile, which would mask the input the user actually supplied.
    let bytes = name.as_bytes();
    if is_c_locale_space(bytes[0]) || is_c_locale_space(bytes[bytes.len() - 1]) {
        return false;
    }

    // Reject path separators and absolute paths (drive letters, leading slash).
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    if Path::new(name).is_absolute() {
        return false;
    }

    // Reject "." and "..".
    if name == "." || name == ".." {
        return false;
    }

    // Reject trailing '.' (CreateFile strips it silently on Windows).
    if name.ends_with('.') {
        return false;
    }

    // Reject Windows reserved device names. Compare against the lowercase
    // stem (everything before the final '.') so "CON", "con", and "CON.txt"
    // are all rejected. Mirrors the kReservedNames set in the C++ source.
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

/// Lexical normalization of a path, mirroring C++
/// `std::filesystem::path::lexically_normal` for the cases the containment
/// check needs: drop `.` components, fold `name/..` pairs, drop `..` directly
/// after a root directory, and turn an all-elided non-empty input into `.`.
/// (C++ preserves a trailing separator as a trailing empty element; Rust
/// component iteration ignores trailing separators, which does not affect the
/// component-wise comparison in [`is_inside`].)
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
                    // Leading ".." in a relative path is preserved; ".."
                    // directly after a root directory is dropped.
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

/// Weakly-canonical equivalent of C++
/// `std::filesystem::weakly_canonical(p, ec)`.
///
/// `std::fs::canonicalize` fails on nonexistent paths, while `weakly_canonical`
/// does not. This port canonicalizes the deepest existing prefix and appends
/// the nonexistent remainder lexically normalized (the same shrink-from-the-end
/// strategy as the MSVC STL). Errors other than "does not exist" propagate,
/// mirroring the C++ error-code path; a path with no existing prefix at all
/// resolves to its lexically normal form.
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
    // Try progressively shorter leading prefixes until one canonicalizes.
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
    // Nothing exists: purely lexical result, as weakly_canonical produces.
    Ok(lexically_normal(p))
}

/// Validate that `child` resolves to a location inside `parent` (no
/// traversal). Mirror of `mo2core::is_inside`.
///
/// The C++ computes `weakly_canonical(child).lexically_relative(
/// weakly_canonical(parent))` and requires the result to be non-empty and not
/// start with `..`. Both weakly-canonical results are in lexically normal
/// form, so that reduces to a component-wise prefix test; note that
/// `child == parent` yields `.` in C++, which passes, so equality is "inside"
/// here too. Canonicalization errors are treated as `false`, exactly as the
/// C++ swallows the error codes.
pub fn is_inside(parent: &Path, child: &Path) -> bool {
    let Ok(canonical_child) = weakly_canonical(child) else {
        return false;
    };
    let Ok(canonical_parent) = weakly_canonical(parent) else {
        return false;
    };
    canonical_child.starts_with(&canonical_parent)
}

/// Directory of the host executable. Mirror of
/// `mo2core::executable_directory` (`GetModuleFileNameW(nullptr, ...)`).
///
/// Use this for resources tied to a specific executable. Do not use it for
/// resources that should follow the calling binary inside MO2; use
/// [`module_directory`] for those (inside MO2 the host EXE is
/// `ModOrganizer.exe`, so this function would point at MO2's install root).
///
/// Falls back to the current working directory if the Win32 lookup fails or
/// the platform is not Windows.
pub fn executable_directory() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(dir) = win::module_path_for(std::ptr::null_mut()) {
            return dir;
        }
    }
    current_dir_fallback()
}

/// Directory of the module containing `anchor`. Mirror of
/// `mo2core::module_directory`.
///
/// On Windows, resolves to the directory of the DLL or EXE the address lives
/// in (via `GetModuleHandleExW` with `GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS`),
/// regardless of the host process's working directory or the host EXE's
/// location. Use this for resources owned by the binary the code itself is in
/// (logs next to `mo2_salma_rs.dll`). Falls back to the current working
/// directory if the lookup fails or the platform is not Windows.
///
/// `anchor` is any address inside the module to query; a function pointer to
/// a symbol defined in this crate is sufficient, e.g.
/// `module_directory(module_directory as *const c_void)`.
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

/// Shared fallback for the directory lookups. The C++ falls back to
/// `std::filesystem::current_path()`, which throws on failure; this port maps
/// that (practically unreachable) failure to `"."` instead of panicking.
fn current_dir_fallback() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(windows)]
mod win {
    //! Win32 module lookups, mirror of the `#ifdef _WIN32` block in
    //! `src/Utils.cpp`.

    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{HMODULE, MAX_PATH};
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };

    /// Resolve the parent directory of the file backing `hmod`. Pass null to
    /// query the host executable. Returns `None` on lookup failure; callers
    /// fall back to the current working directory. Mirror of the C++
    /// `module_path_for`, including the buffer-doubling retry loop capped at
    /// `kMaxRetries = 5`.
    pub(super) fn module_path_for(hmod: HMODULE) -> Option<PathBuf> {
        const MAX_RETRIES: u32 = 5;
        let mut buf: Vec<u16> = vec![0; MAX_PATH as usize];
        // SAFETY: buf is a valid, writable u16 buffer of the length passed.
        let mut len = unsafe { GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32) };
        let mut retries = 0;
        while len as usize >= buf.len() && retries < MAX_RETRIES {
            buf.resize(buf.len() * 2, 0);
            // SAFETY: buf was just resized; pointer and length stay in sync.
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

    /// `GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, anchor, ...)`
    /// then [`module_path_for`]. Mirror of the Windows branch of the C++
    /// `module_directory`.
    pub(super) fn module_directory_for_address(
        anchor: *const core::ffi::c_void,
    ) -> Option<PathBuf> {
        let mut hmod: HMODULE = std::ptr::null_mut();
        // SAFETY: anchor is only inspected as an address (FROM_ADDRESS flag);
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

    // Ported 1:1 from tests/utils_test.cpp (59 TEST()/TEST_F() cases, same
    // inputs and expected outputs, names converted to snake_case). The
    // pugixml-based fixtures (GetOrderedNodesTest, XmlBoolAttributeTest) map
    // to the generic string-based ports; see PARITY-NOTES "Task 3".

    // --- to_lower ---

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

    // --- normalize_path ---

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

    // --- random_hex_string ---

    #[test]
    fn random_hex_string_default_length() {
        // C++ calls random_hex_string() and relies on the default argument
        // (12); Rust has no default arguments, so the default is a constant.
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
        // Statistically near-impossible to collide at length 32.
        let a = random_hex_string(32);
        let b = random_hex_string(32);
        assert_ne!(a, b);
    }

    // --- get_ordered_nodes ---
    //
    // The C++ TEST_F fixtures parse XML documents; the generic port takes the
    // pre-read order attribute plus (name, doc_index) pairs in document order.
    // Same inputs (names and document positions) and expected outputs.

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

    // --- xml_bool_attribute_true ---
    //
    // The C++ TEST_F fixtures parse XML attributes; the generic port takes
    // None for a missing attribute and Some(value) otherwise.

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

    // --- parse_plugin_type_string ---

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

    // --- is_safe_mod_name ---

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

    // Containment invariant: even if a hostile name slipped past
    // is_safe_mod_name, the defense-in-depth is_inside check on the joined
    // path catches it. This exercises the integration the upload controller
    // relies on. (Directory name differs from the C++ test's so the two
    // suites cannot collide when run concurrently.)
    #[test]
    fn is_inside_rejects_mod_name_traversal_generated_path_stays_inside_mods_dir() {
        let tmp = std::env::temp_dir().join("salma_rs_modname_containment_test");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        assert!(is_inside(&tmp, &tmp.join("SkyUI")));
        assert!(!is_inside(&tmp, &tmp.join("..").join("escape")));
        std::fs::remove_dir_all(&tmp).expect("remove temp dir");
    }

    // --- Rust-only additions below (not part of the C++ utils_test.cpp) ---

    /// FNV-1a-64 known vectors (published FNV test values), pinning the
    /// offset basis / prime / byte order.
    #[test]
    fn fnv1a_hash_known_vectors() {
        assert_eq!(fnv1a_hash(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a_hash(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a_hash(b"foobar"), 0x85944171f73967e8);
    }

    /// fnv1a_hash is const-evaluable, mirroring the C++ constexpr (used for
    /// the "..."_h dispatch pattern, which Rust replaces with match).
    #[test]
    fn fnv1a_hash_const_eval() {
        const H: u64 = fnv1a_hash(b"flagDependency");
        assert_eq!(H, fnv1a_hash(b"flagDependency"));
    }

    /// Fixture-driven FNV check against two committed golden ModuleConfig.xml
    /// files. Expected constants were computed once from the committed bytes
    /// with the harness's reference implementation:
    ///
    /// ```text
    /// python -c "import sys; sys.path.insert(0,'rust/tools'); import gen_golden;
    ///   print(gen_golden.fnv1a_hex(open(r'rust/tests/golden/cases/<case>/ModuleConfig.xml','rb').read()))"
    /// ```
    ///
    /// zip_exactlyone_mu_joint_fix -> 91d7af004eea83d1 (2876 bytes)
    /// sevenz_3step_tk_dodge       -> 777a83236a2c8541 (5910 bytes)
    #[test]
    fn fnv1a_hash_matches_golden_fixture_hashes() {
        let cases: &[(&str, u64, usize)] = &[
            ("zip_exactlyone_mu_joint_fix", 0x91d7af004eea83d1, 2876),
            ("sevenz_3step_tk_dodge", 0x777a83236a2c8541, 5910),
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cases");
        for (case, expected_hash, expected_len) in cases {
            let path = root.join(case).join("ModuleConfig.xml");
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
            assert_eq!(
                bytes.len(),
                *expected_len,
                "fixture byte length drifted: {case}"
            );
            assert_eq!(fnv1a_hash(&bytes), *expected_hash, "fnv1a mismatch: {case}");
        }
    }

    /// hash_combine reference values, computed once with a python model of
    /// the exact C++ formula in src/FomodCSPPrecompute.cpp
    /// (seed ^= v + 0x9e3779b97f4a7c15 + (seed << 6) + (seed >> 2), mod 2^64).
    #[test]
    fn hash_combine_matches_cpp_formula() {
        let mut seed = 0u64;
        hash_combine(&mut seed, 0);
        assert_eq!(seed, 0x9e3779b97f4a7c15);

        let mut seed = 14695981039346656037u64;
        hash_combine(&mut seed, 42);
        assert_eq!(seed, 0x0629c6f72cf9ed6d);

        // Chained combine over fnv1a values, the CSP signature usage pattern.
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
        // "\" or "/" mean mod root, not an absolute path.
        assert_eq!(normalize_destination_for_join("/"), "");
        assert_eq!(normalize_destination_for_join("\\"), "");
        assert_eq!(normalize_destination_for_join("//dir/file"), "dir/file");
        assert_eq!(normalize_destination_for_join("./dir"), "dir");
        assert_eq!(normalize_destination_for_join(".\\dir"), "dir");
        assert_eq!(normalize_destination_for_join("././dir"), "dir");
        assert_eq!(normalize_destination_for_join("dir/file"), "dir/file");
    }

    /// The strip loops run sequentially (slashes first, then "./"), so a
    /// slash re-exposed by the "./" strip survives - C++ quirk replicated
    /// exactly (see PARITY-NOTES).
    #[test]
    fn normalize_destination_for_join_sequential_strip_quirk() {
        assert_eq!(normalize_destination_for_join(".//foo"), "/foo");
    }

    #[test]
    fn resolve_file_destination_file_rules() {
        // Empty destination for a <file>: install to the source filename.
        assert_eq!(
            resolve_file_destination("a/b/plugin.esp", "", true),
            "plugin.esp"
        );
        assert_eq!(
            resolve_file_destination("plugin.esp", "", true),
            "plugin.esp"
        );
        // Trailing-slash destination for a <file>: treat as directory.
        assert_eq!(
            resolve_file_destination("a\\b\\plugin.esp", "dest/", true),
            "dest/plugin.esp"
        );
        assert_eq!(
            resolve_file_destination("a/plugin.esp", "dest\\", true),
            "dest\\plugin.esp"
        );
        // Plain destination passes through the join-normalizer.
        assert_eq!(
            resolve_file_destination("a/plugin.esp", "/dest.esp", true),
            "dest.esp"
        );
    }

    #[test]
    fn resolve_file_destination_folder_rules() {
        // <folder> destinations pass through unchanged; empty stays empty
        // (folder contents land at the mod root).
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
        // Traversal-only inputs normalize to empty, which resolves to the
        // mod root and is safe (normalize_path already dropped the "..").
        assert!(is_safe_destination(".."));
        assert!(is_safe_destination("../.."));
        assert!(is_safe_destination("a/../b"));
        // Drive letters are rejected after normalization.
        assert!(!is_safe_destination("C:/evil"));
        assert!(!is_safe_destination("c:\\evil"));
    }

    #[test]
    fn is_inside_accepts_equal_paths_like_cpp_dot_relative() {
        // C++ lexically_relative(p, p) == "." which does not start with "..",
        // so a path is considered inside itself.
        let tmp = std::env::temp_dir();
        assert!(is_inside(&tmp, &tmp));
    }

    #[test]
    fn is_inside_rejects_sibling_with_common_prefix() {
        // "C:/foo" vs "C:/foobar": component-wise, not string-prefix-wise.
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
        // Existing prefix canonicalizes; nonexistent tail appends lexically.
        let tmp = std::env::temp_dir();
        let child = tmp.join("salma_rs_wc_nonexistent").join("sub");
        let wc = weakly_canonical(&child).expect("weakly_canonical");
        assert!(wc.ends_with(Path::new("salma_rs_wc_nonexistent/sub")));
        // Fully nonexistent relative path resolves lexically.
        let lex = weakly_canonical(Path::new("salma_rs_no_such_dir/x/../y")).expect("lexical");
        assert_eq!(lex, Path::new("salma_rs_no_such_dir/y"));
    }

    #[test]
    fn executable_directory_exists() {
        let dir = executable_directory();
        assert!(dir.is_dir(), "not a directory: {}", dir.display());
    }

    /// In a test binary the code is statically linked into the executable, so
    /// the module owning any function address is the test exe itself and the
    /// two lookups must agree.
    #[test]
    fn module_directory_of_local_symbol_is_executable_directory() {
        let anchor = module_directory as *const core::ffi::c_void;
        assert_eq!(module_directory(anchor), executable_directory());
    }
}
