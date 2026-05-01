//! Shared helpers: byte-level string and path normalization, FNV-1a hashing,
//! FOMOD attribute parsing, path-safety screens, and module directory lookups.
//!
//! Four things to know before editing anything here.
//!
//! - Everything is byte-level and ASCII-only. [`to_lower`] touches `A`-`Z` and
//!   nothing else, so every comparison built on it is a byte comparison, never
//!   a Unicode case-fold or a locale collation.
//! - [`normalize_path`] and [`normalize_destination_for_join`] look
//!   interchangeable and are not. They run the same two strip loops in opposite
//!   orders and disagree on inputs like `.//foo`. Each item doc gives its own
//!   order. Do not unify them.
//! - [`is_safe_destination`] and [`is_safe_mod_name`] are string screens, not
//!   containment guarantees. Both accept inputs that escape the intended root
//!   once joined. A caller that needs a containment guarantee must also run
//!   [`is_inside`] on the joined path: `is_safe_mod_name`'s caller does, while
//!   `fomod_service::enqueue_entry` deliberately does not and reproduces the
//!   resulting hole. The two item docs are authoritative on each screen's
//!   contract and name the exact hole.
//! - [`get_ordered_nodes`] and [`xml_bool_attribute_true`] are generic over
//!   plain strings rather than over an XML type, so the XML library stays out
//!   of this module. [`crate::fomod_ir_parser`] adapts roxmltree's node and
//!   attribute types onto those signatures.
//!
//! Several rules here look like bugs and are deliberate. For the path and
//! string rules the reason is one and the same: the installed mod layouts this
//! engine has to reproduce were produced by exactly these rules. That reason
//! does not extend to the Win32 lookups in the private `win` module, whose one
//! quirk states its own. Each case is marked at its site. See
//! `PARITY-NOTES.md`.

use crate::types::PluginType;
use std::path::{Component, Path, PathBuf};

/// Lowercase a string at the byte level.
///
/// Only ASCII `A`-`Z` change. Bytes >= 0x80 pass through, and every byte of a
/// multi-byte UTF-8 sequence is >= 0x80, so non-ASCII text is untouched. This
/// is a byte-level lowercaser, not a Unicode case-folder.
pub fn to_lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Normalize an archive or mod path.
///
/// Six stages, in this order:
///
/// 1. lowercase ([`to_lower`])
/// 2. backslash to forward slash
/// 3. strip leading `./` prefixes, then strip leading `/` prefixes
/// 4. strip trailing `/`
/// 5. single-pass collapse of consecutive slashes
/// 6. drop `.` and `..` segments (syntactic strip, no filesystem resolution)
///
/// The output is lowercase, uses forward slashes only, has no leading or
/// trailing slash, no repeated `/`, and no `.` or `..` segment.
///
/// Stage 3 is two sequential loops, not one combined loop, and their order
/// decides the result: the `./` loop can re-expose a slash, and the `/` loop
/// runs after it and eats that slash. One trace through all six stages:
///
/// ```text
///   input          ./\Textures\\LOD/
///   1 to_lower     ./\textures\\lod/
///   2 \ -> /       .//textures//lod/
///   3a strip ./    /textures//lod/     one "./" consumed, a slash re-exposed
///   3b strip /     textures//lod/      the second loop then eats it
///   4 trim trail   textures//lod
///   5 collapse //  textures/lod
///   6 drop . ..    textures/lod
/// ```
///
/// [`normalize_destination_for_join`] runs the same two loops in the opposite
/// order and so does not clean up after itself: `.//foo` gives `/foo` there and
/// `foo` here. The difference is deliberate. Do not unify the two.
pub fn normalize_path(p: &str) -> String {
    let lowered = to_lower(p).replace('\\', "/");
    // Strip leading "./" or "/" prefixes that some archivers emit. Two
    // sequential loops, "./" first: see the item doc.
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

/// FNV-1a 64-bit hash.
///
/// `h0 = 0xCBF29CE484222325`; `h_{i+1} = (h_i XOR b_i) * 0x100000001B3`, with
/// u64 wrapping multiplication, fed in byte-stream order. `const fn`, so it can
/// seed compile-time constants.
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

/// Boost-style hash combiner:
///
/// `seed ^= v + 0x9e3779b97f4a7c15 + (seed << 6) + (seed >> 2)`
///
/// with u64 wrapping addition throughout. `seed` is updated in place.
///
/// It lives here because three CSP modules need it:
/// [`crate::fomod_csp_precompute`] (flag-state signatures),
/// [`crate::fomod_csp_options`] (option signatures) and
/// [`crate::fomod_csp_solver`] (memo keys).
///
/// The fold is not commutative: combining the same values in a different order
/// generally gives a different seed. A caller that needs an order-free
/// signature must fold its inputs in a fixed order, sorted for example, which
/// is what the CSP signature helpers do.
pub fn hash_combine(seed: &mut u64, v: u64) {
    *seed ^= v
        .wrapping_add(0x9e3779b97f4a7c15)
        .wrapping_add(*seed << 6)
        .wrapping_add(*seed >> 2);
}

/// Length callers pass to [`random_hex_string`] for an ordinary scratch-name
/// token: 12 characters.
pub const RANDOM_HEX_DEFAULT_LEN: usize = 12;

/// Generate a random lowercase-hex string of exactly `length` characters.
///
/// The alphabet is `0-9a-f`. The source is a thread-local SplitMix64 stream,
/// seeded once per thread from std's OS-seeded hasher entropy mixed with the
/// system clock, so no external `rand` dependency is needed. It is not
/// cryptographic: use the output as a uniqueness token for scratch names, never
/// as a secret.
pub fn random_hex_string(length: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    use std::cell::Cell;
    thread_local! {
        // Per-thread state avoids contention when several threads extract
        // concurrently.
        static RNG_STATE: Cell<u64> = Cell::new(random_seed());
    }
    RNG_STATE.with(|state| {
        let mut out = String::with_capacity(length);
        for _ in 0..length {
            // One SplitMix64 step per character.
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

/// Build a per-thread seed from std's OS-seeded hasher entropy plus the system
/// clock.
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
    /// Ascending byte-wise lexicographic order on the `name` attribute (the
    /// schema default). Case-sensitive: uppercase ASCII sorts before
    /// lowercase, so `"Banana"` comes before `"apple"`.
    Ascending,
    /// Descending byte-wise lexicographic order on the `name` attribute.
    /// Case-sensitive in the same way as [`NodeOrder::Ascending`].
    Descending,
    /// Document order (no sort).
    Explicit,
}

/// Map an `order` attribute value to a [`NodeOrder`]:
///
/// - a missing attribute defaults to `"Ascending"`
/// - `"Descending"` sorts descending, `"Ascending"` sorts ascending
/// - anything else, including `"Explicit"`, unknown values and any casing
///   mismatch, falls through to document order
///
/// The match arms are exact, case-sensitive string comparisons with no trimming
/// and no case folding, so `"ascending"` and `" Ascending"` both land on
/// [`NodeOrder::Explicit`]. That fall-through is the intended behavior, not an
/// oversight.
pub fn parse_node_order(order_attr: Option<&str>) -> NodeOrder {
    match order_attr.unwrap_or("Ascending") {
        "Descending" => NodeOrder::Descending,
        "Ascending" => NodeOrder::Ascending,
        _ => NodeOrder::Explicit,
    }
}

/// Reorder sibling nodes per the FOMOD `order` attribute.
///
/// Takes the already-read attribute value, the children in document order, and
/// a projection to each child's `name`. [`crate::fomod_ir_parser`] supplies the
/// roxmltree adapter that fills those three arguments. `nodes` comes back
/// reordered; no element is added or removed.
///
/// Ordering guarantees, which feed straight into the inference grid:
///
/// - The comparison is `str::cmp`: byte-wise lexicographic over the UTF-8 bytes
///   of the projected name, case-sensitive, so `"Banana"` sorts before
///   `"apple"`. This is neither alphabetical order nor a locale collation.
/// - Sorting is `sort_unstable_by`, so two nodes sharing a name under Ascending
///   or Descending land in an unspecified relative order.
/// - [`NodeOrder::Explicit`] performs no sort at all and preserves document
///   order exactly.
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

/// Parse an XML boolean attribute using XML Schema semantics. Pass `None` for a
/// missing attribute and `Some(value)` otherwise; [`crate::fomod_ir_parser`]
/// feeds roxmltree attribute values in.
///
/// True for `"true"` and `"1"` only, compared after ASCII-lowercasing with
/// [`to_lower`]. Everything else is false, including a missing attribute, the
/// empty string, `"yes"`, and any value with surrounding whitespace. There is
/// no error path: an unparsable value is simply false.
pub fn xml_bool_attribute_true(attr: Option<&str>) -> bool {
    let Some(raw) = attr else {
        return false;
    };
    let value = to_lower(raw);
    value == "true" || value == "1"
}

/// Map a FOMOD plugin type name to its [`PluginType`]. Unrecognized names,
/// including the empty string, default to [`PluginType::Optional`].
pub fn parse_plugin_type_string(type_name: &str) -> PluginType {
    match type_name {
        "Required" => PluginType::Required,
        "Recommended" => PluginType::Recommended,
        "Optional" => PluginType::Optional,
        "NotUsable" => PluginType::NotUsable,
        "CouldBeUsable" => PluginType::CouldBeUsable,
        // Unknown type names install as Optional.
        _ => PluginType::Optional,
    }
}

/// Map a [`PluginType`] to its FOMOD type name. The match is exhaustive, so
/// every value has a name and there is no fallback string.
pub fn plugin_type_to_string(plugin_type: PluginType) -> &'static str {
    match plugin_type {
        PluginType::Required => "Required",
        PluginType::Recommended => "Recommended",
        PluginType::Optional => "Optional",
        PluginType::NotUsable => "NotUsable",
        PluginType::CouldBeUsable => "CouldBeUsable",
    }
}

/// Strip leading slashes and `./` from a FOMOD destination so it is safe to
/// join onto a mod-root directory path.
///
/// FOMOD destinations are mod-root-relative: a value like `\` or `/` means
/// "root", not an absolute filesystem path. Only leading separators are
/// stripped; a separator anywhere else is left exactly as written.
///
/// The two strip loops run in this order: all leading slashes first, then `./`
/// and `.\` prefixes. Nothing runs after the second loop, so a slash that the
/// `./` strip re-exposes survives, and `.//foo` returns `/foo`. That looks like
/// a bug and is load-bearing: the installed layouts this engine reproduces were
/// produced by exactly this order. It is harmless downstream because the FOMOD
/// IR parser re-normalizes with [`normalize_path`], which runs the same two
/// loops the other way round and does clean up. Do not reorder the loops here.
/// See `PARITY-NOTES.md`.
pub fn normalize_destination_for_join(destination: &str) -> String {
    let mut s = destination.trim_start_matches(['\\', '/']);
    while let Some(rest) = s.strip_prefix("./").or_else(|| s.strip_prefix(".\\")) {
        s = rest;
    }
    s.to_string()
}

/// Resolve a `<file>` or `<folder>` node's destination, handling empty
/// destinations and trailing-slash directory semantics, then normalize it for a
/// filesystem join. All five branches:
///
/// ```text
///   is_file | raw_destination      | intermediate result
///   --------+----------------------+------------------------------------
///    true   | ""                   | filename(source)
///    true   | ends with / or \     | raw_destination + filename(source)
///    true   | anything else        | raw_destination
///    false  | ""                   | ""   (folder contents land at mod root)
///    false  | anything else        | raw_destination
///
///   then: normalize_destination_for_join(intermediate result)
/// ```
///
/// `filename(source)` is everything after the last `/` or `\` in `source`, or
/// the whole of `source` when it holds no separator.
///
/// The trailing-separator arm keeps the separator the caller wrote, so a
/// `dest\` destination yields `dest\plugin.esp`, backslash included.
/// [`normalize_destination_for_join`] strips leading separators only and does
/// not convert that one. A caller that needs forward slashes must run
/// [`normalize_path`] itself.
pub fn resolve_file_destination(source: &str, raw_destination: &str, is_file: bool) -> String {
    let mut destination = raw_destination.to_string();
    if is_file && destination.is_empty() {
        destination = source_filename(source).to_string();
    } else if is_file && destination.ends_with(['/', '\\']) {
        destination.push_str(source_filename(source));
    }
    normalize_destination_for_join(&destination)
}

/// Filename part of a source path: everything after the last `/` or `\`, or the
/// whole string when it holds no separator.
fn source_filename(source: &str) -> &str {
    match source.rfind(['/', '\\']) {
        Some(pos) => &source[pos + 1..],
        None => source,
    }
}

/// Screen a FOMOD destination string. Returns true for "accepted".
///
/// What the check does, in order:
///
/// 1. An empty input is accepted; it resolves to the mod root.
/// 2. The input goes through [`normalize_path`], which strips every `.` and
///    `..` segment, so no traversal sequence survives into the normalized form.
///    An input made only of those segments normalizes to empty and is accepted,
///    again as the mod root.
/// 3. The normalized form is rejected when its second byte is `:`, which
///    catches a Windows drive letter such as `C:/evil`. This is the only live
///    rejection.
///
/// The leading-`/` arm in the code is unreachable, because step 2 already
/// stripped every leading slash before the first byte is tested. So
/// `is_safe_destination("/etc/passwd")` returns true. Keep the dead branch and
/// do not describe it as protection; `PARITY-NOTES.md` records why it stays.
///
/// **Caller contract.** This is a string screen, not a containment check. It
/// says nothing about the path the caller builds from the string, and the one
/// caller that matters joins the raw destination rather than the normalized
/// one. A rooted destination such as `/etc/passwd` passes the screen and then
/// discards the base during the join, because a root component wins in
/// `Path::join`. On Windows only the base's drive prefix survives:
/// `Path::new(r"D:\mods").join("/etc/passwd")` is `D:/etc/passwd`. A caller
/// that needs a containment guarantee must also run [`is_inside`] on the joined
/// path. `fomod_service::enqueue_entry` deliberately does not, and reproduces
/// the hole rather than closing it; that decision is documented there.
pub fn is_safe_destination(dest: &str) -> bool {
    if dest.is_empty() {
        return true;
    }
    let norm = normalize_path(dest);
    if norm.is_empty() {
        return true;
    }
    let bytes = norm.as_bytes();
    // `bytes[0] == b'/'` is dead: normalize_path already stripped every leading
    // slash, so only the drive-letter test can fire. Kept on purpose; see
    // PARITY-NOTES.md.
    if bytes[0] == b'/' || (bytes.len() >= 2 && bytes[1] == b':') {
        return false;
    }
    true
}

/// The C-locale `isspace` set: space, `\t`, `\n`, `\v`, `\f`, `\r`. Bytes of
/// 0x80 and above never count, so multi-byte UTF-8 is unaffected.
/// `u8::is_ascii_whitespace` omits `\v`, so it is not a drop-in replacement.
fn is_c_locale_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Screen a mod-name string for use as a single directory component under a
/// mods root. Returns true for "accepted". Rejects a name that:
///
/// - is empty
/// - starts or ends with C-locale whitespace (space, `\t`, `\n`, `\v`, `\f`,
///   `\r`)
/// - contains the path separators `/` or `\`
/// - parses as an absolute path (defensive only; unreachable once separators
///   are rejected, since every absolute path contains one)
/// - equals `.` or `..`
/// - ends with `.`, which `CreateFile` strips silently on Windows
/// - has a lowercase stem, meaning everything before the final `.`, matching a
///   Windows reserved device name: CON, PRN, AUX, NUL, COM1-9, LPT1-9
///
/// **Limit of the check.** A Windows drive-relative name such as `C:` or
/// `C:evil` is accepted: it holds no separator, and `Path::is_absolute` is
/// false for a path with a prefix but no root. Joining such a name onto a mods
/// root discards the root, because on Windows a component with a prefix and no
/// root replaces the whole path. `Path::new(r"D:\mods").join("C:")` is `C:`,
/// not `D:\mods\C:`.
///
/// So this is a screen, not a containment guarantee. Every caller must also run
/// [`is_inside`] on the joined path. `InstallationController.cpp` does exactly
/// that: `is_safe_mod_name` on the request field, then `is_inside` on the path
/// it built, with the second check deciding.
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
    // are all rejected.
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

/// Normalize a path lexically, without touching the filesystem: drop `.`
/// components, fold `name/..` pairs, drop `..` directly after a root directory,
/// keep a leading `..` in a relative path, and turn an all-elided non-empty
/// input into `.`. An empty input stays empty.
///
/// Component iteration ignores a trailing separator, which does not affect the
/// component-wise comparison in [`is_inside`].
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

/// Canonicalize a path that does not have to exist.
///
/// `std::fs::canonicalize` fails on a nonexistent path. This shrinks from the
/// end instead: it canonicalizes the deepest existing prefix, then appends the
/// remainder in lexically normal form. A path with no existing prefix at all
/// resolves to its lexically normal form.
///
/// ```text
///   p = D:\mods\SkyUI\does\not\exist
///
///   canonicalize(D:\mods\SkyUI\does\not\exist)  -> NotFound
///   canonicalize(D:\mods\SkyUI\does\not)        -> NotFound
///   canonicalize(D:\mods\SkyUI\does)            -> NotFound
///   canonicalize(D:\mods\SkyUI)                 -> OK \\?\D:\mods\SkyUI
///                                                  push does\not\exist
///                                                  then lexically_normal
///
///   nothing canonicalizes -> lexically_normal(p), no syscall result used
/// ```
///
/// **I/O and errors.** This performs blocking filesystem I/O: one
/// `canonicalize` call for the full path, then up to one more per shorter
/// leading prefix, so a fully nonexistent path of N components costs N
/// syscalls. Canonicalization follows symlinks, so the answer depends on
/// filesystem state at the moment of the call.
///
/// Two error kinds are swallowed and drive the shrink: `NotFound` and
/// `NotADirectory`. `NotADirectory` is grouped with `NotFound` because a
/// regular file in the middle of the path (`mods/readme.txt/sub`) means the
/// remainder cannot exist either, so shrinking is still the right move. Every
/// other error kind propagates as `Err`, and [`is_inside`] turns that `Err`
/// into `false`.
///
/// On Windows a successful result carries the `\\?\` verbatim prefix that
/// `std::fs::canonicalize` produces. [`is_inside`] compares two such results
/// against each other, so the prefix is on both sides and containment is
/// unaffected.
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
    // Nothing exists: purely lexical result.
    Ok(lexically_normal(p))
}

/// Validate that `child` resolves to a location inside `parent`, with no
/// traversal out of it.
///
/// Both sides go through [`weakly_canonical`], which yields lexically normal
/// paths, so containment reduces to a component-wise prefix test. Two
/// consequences worth knowing: `child == parent` counts as inside, and the test
/// is component-wise rather than string-prefix-wise, so `C:/foobar` is not
/// inside `C:/foo`.
///
/// This is a security predicate that touches the disk. Its operational
/// contract:
///
/// - It performs blocking filesystem I/O through [`weakly_canonical`], up to
///   one `canonicalize` syscall per path component for a path that does not
///   exist. Do not call it in a tight loop over untrusted input.
/// - It fails closed: any canonicalization error on either side returns
///   `false`.
/// - It resolves symlinks and reflects filesystem state at the moment of the
///   call, so the answer is point-in-time and TOCTOU-sensitive. A path that is
///   inside now can be outside by the time the caller opens it. Use it as a
///   screen before an operation, not as a substitute for opening the file
///   safely.
/// - Neither argument has to exist. A nonexistent path resolves as far as it
///   can and the remainder is treated lexically.
/// - On Windows both sides are compared as `\\?\` verbatim paths, so the prefix
///   does not affect containment.
pub fn is_inside(parent: &Path, child: &Path) -> bool {
    let Ok(canonical_child) = weakly_canonical(child) else {
        return false;
    };
    let Ok(canonical_parent) = weakly_canonical(parent) else {
        return false;
    };
    canonical_child.starts_with(&canonical_parent)
}

/// Directory of the host executable (`GetModuleFileNameW(nullptr, ...)`).
///
/// Use this for resources tied to the running program. Do not use it for
/// resources that should follow the calling binary inside MO2: there the host
/// EXE is `ModOrganizer.exe`, so this points at MO2's install root. Use
/// [`module_directory`] instead.
///
/// Falls back to the current working directory when the Win32 lookup fails or
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

/// Directory of the module containing `anchor`.
///
/// On Windows this resolves to the directory of the DLL or EXE the address
/// lives in, via `GetModuleHandleExW` with
/// `GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS`, regardless of the host process's
/// working directory or the host EXE's location. Use it for resources owned by
/// the binary the code itself is in, such as the log next to
/// `mo2_salma_rs.dll`. Falls back to the current working directory when the
/// lookup fails or the platform is not Windows.
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

/// Shared fallback for the directory lookups: the current working directory,
/// or `"."` when even that cannot be read. Never panics.
fn current_dir_fallback() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(windows)]
mod win {
    //! Win32 module path lookups behind [`super::executable_directory`] and
    //! [`super::module_directory`].

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
    /// then fall back to the current working directory.
    ///
    /// The buffer starts at `MAX_PATH` (260 wide chars) and doubles on each
    /// retry, capped at `MAX_RETRIES = 5`, so the last attempt reaches 8320
    /// wide chars.
    ///
    /// `None` comes back in three cases: the Win32 call reported length 0, the
    /// path still needs more room after five doublings, and, less obviously,
    /// the path first fits on the fifth doubling. That last result is discarded
    /// even though the call returned a complete path, because the success test
    /// re-checks `retries < MAX_RETRIES` and `retries` is already 5 by then.
    ///
    /// The third case looks like a bug and is deliberate. The retry loop and
    /// its discard on `len == 0` or exhausted retries are reproduced as
    /// recorded in `PARITY-NOTES.md`, and there is no other justification for
    /// it. Relaxing the `retries` bound in the success test changes what both
    /// callers resolve for a module path that long, from the current working
    /// directory to the real module directory, so it is a behavior change and
    /// not a cleanup.
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

    /// `GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, anchor, ...)`,
    /// then [`module_path_for`]. `None` if the handle lookup fails.
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
    // Inputs are the pre-read order attribute plus (name, document index)
    // pairs in document order, standing in for parsed XML children.

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
    // None stands for a missing attribute, Some(value) for a present one.

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

    // Containment invariant: even when a hostile name slips past
    // is_safe_mod_name, the is_inside check on the joined path catches it.
    // This exercises the pairing the upload controller relies on. The temp
    // directory name is unique to this test so concurrent suites cannot
    // collide.
    #[test]
    fn is_inside_rejects_mod_name_traversal_generated_path_stays_inside_mods_dir() {
        let tmp = std::env::temp_dir().join("salma_rs_modname_containment_test");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        assert!(is_inside(&tmp, &tmp.join("SkyUI")));
        assert!(!is_inside(&tmp, &tmp.join("..").join("escape")));
        std::fs::remove_dir_all(&tmp).expect("remove temp dir");
    }

    // --- hashing, path resolution and module lookups ---

    /// Published FNV-1a-64 test vectors, pinning the offset basis, the prime
    /// and the byte order.
    #[test]
    fn fnv1a_hash_known_vectors() {
        assert_eq!(fnv1a_hash(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a_hash(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a_hash(b"foobar"), 0x85944171f73967e8);
    }

    /// fnv1a_hash is const-evaluable, so it can seed compile-time constants.
    #[test]
    fn fnv1a_hash_const_eval() {
        const H: u64 = fnv1a_hash(b"flagDependency");
        assert_eq!(H, fnv1a_hash(b"flagDependency"));
    }

    /// Further FNV-1a-64 vectors, each computed with an independent Python
    /// implementation of the same algorithm (offset basis
    /// 14695981039346656037, prime 1099511628211).
    ///
    /// The inputs are inline rather than read off disk, so the test cannot
    /// start passing because a fixture changed underneath it.
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

    /// hash_combine reference values, computed once with a Python model of
    /// `seed ^= v + 0x9e3779b97f4a7c15 + (seed << 6) + (seed >> 2)` mod 2^64.
    /// These constants are the reference for the formula.
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

    /// The strip loops run in sequence, slashes first and then "./", so a
    /// slash re-exposed by the "./" strip survives. Deliberate.
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
        // Drive letters are rejected after normalization, the only live
        // rejection.
        assert!(!is_safe_destination("C:/evil"));
        assert!(!is_safe_destination("c:\\evil"));
        // Deliberately not asserted here: a rooted destination such as
        // "/etc/passwd" is accepted, because normalize_path strips the leading
        // slash before the leading-slash branch is reached. The consequence is
        // pinned by fomod_service's
        // enqueue_entry_reproduces_the_rooted_destination_hole.
    }

    #[test]
    fn is_inside_accepts_equal_paths_like_cpp_dot_relative() {
        // A path counts as inside itself.
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
