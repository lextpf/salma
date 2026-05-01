//! Archive listing, reading and extraction behind one facade.
//!
//! Three backends, picked by file extension:
//!
//! - `zip` for ZIP: central-directory order, forward-slash paths.
//! - `sevenz_rust2` for `.7z` and `.001`: solid-block aware, header-only list.
//! - `unrar` for RAR: sequential; links the proprietary unRAR C sources.
//!
//! Entry order and path separators differ per backend and are load-bearing.
//! Later stages consume a listing in the order it arrives and match entry paths
//! against an already-installed mod tree, so reordering or re-separating a
//! listing can change which options the engine infers. The rules below are the
//! contract, not an implementation detail.
//!
//! ## Per-format listing rules
//!
//! ```text
//! format | crate        | dir entries | separator  | order
//! -------+--------------+-------------+------------+---------------------------
//! .zip   | zip          | skip is_dir | keep '/'   | central-directory, no sort
//! .7z    | sevenz_rust2 | skip is_dir | '/' -> '\' | ci-sort by backslash path
//! .001   | sevenz_rust2 | as .7z      | as .7z     | as .7z
//! .rar   | unrar        | skip is_dir | native '\' | ci-sort by backslash path
//! ```
//!
//! Extension routing is `format_of`. The rows map to `list_zip`, `list_7z` and
//! `list_rar`, with `sevenz_backslash_name` for the 7z separator conversion and
//! `ci_sort_by_path` for the sort.
//!
//! The case-insensitive sort exists because 7-Zip and WinRAR store entries in
//! case-insensitive order, while `sevenz_rust2` and `unrar` hand back raw header
//! order. Sorting reconciles the two. An archive authored with an unsorted
//! directory diverges from what those tools list.
//!
//! The unit tests at the end of this file cover extension routing, the size
//! cap, the entry-side normalization split, the traversal guard and the ZIP
//! listing shape, so a change to any of the last three fails in-tree first, at
//! `extract_prefix_entry_norm_light_for_zip_full_for_7z_rar` (the split),
//! `extract_rejects_path_traversal_entries` and
//! `safe_output_path_rejects_all_escape_classes` (the guard), or
//! `list_zip_strips_dirs_keeps_order_and_sizes` (the ZIP shape).
//!
//! No test in this file opens a real 7z or RAR archive; every archive test
//! builds an in-memory zip. The 7z and RAR listing, read and extract paths are
//! therefore unbacked in-tree, order and separator rules included, and so is
//! the solid-block drain in `for_each_7z_entry`, where a missing drain surfaces
//! as wrong bytes or a CRC failure rather than a test failure. The Light
//! normalization of the `prefix` argument in
//! [`ArchiveService::extract_prefix`] is unbacked as well. Validate a change to
//! any of them against a live MO2 instance, through `scripts/run_harness.py`
//! and `test_all.py`. `PARITY-NOTES.md` records the measurements that fixed the
//! rules.
//!
//! ## Normalization profiles
//!
//! Two profiles, and they are not interchangeable.
//!
//! - Full is [`normalize_path`]. Six ordered steps: lowercase, `\` -> `/`,
//!   strip leading `./` then leading `/`, strip trailing `/`, collapse repeated
//!   `/`, drop `.` and `..` segments. Its own doc holds the pipeline and a
//!   worked trace.
//! - Light is `to_lower(entry).replace('\\', "/")` and nothing else. It has two
//!   users: the `prefix` argument of [`ArchiveService::extract_prefix`], which
//!   is Light-normalized for every format, and the ZIP entry side of prefix
//!   matching in `prefix_entry_norm`. Switching `prefix_entry_norm` to Full for
//!   every format does not retire the profile; the prefix side stays Light.
//!
//! `sizes` keys and the [`ArchiveService::read_entry`] /
//! [`ArchiveService::read_entries_batch`] match comparisons all use Full.
//!
//! Light differs from Full in four steps, not one. The same entry through both:
//!
//! ```text
//! entry:  ".\Textures//X.dds"
//!
//! Light (the prefix argument for every format; ZIP entries, prefix_entry_norm)
//!   lowercase          -> ".\textures//x.dds"
//!   '\' -> '/'         -> "./textures//x.dds"   <- stops here
//!
//! Full (normalize_path: sizes keys, read_entry, 7z/rar entries)
//!   lowercase          -> ".\textures//x.dds"
//!   '\' -> '/'         -> "./textures//x.dds"
//!   strip leading ./   -> "textures//x.dds"
//!   strip leading /    -> "textures//x.dds"
//!   strip trailing /   -> "textures//x.dds"
//!   collapse //        -> "textures/x.dds"
//!   drop . and .. segs -> "textures/x.dds"
//!
//! prefix "textures":  Light misses, Full matches
//! ```
//!
//! The leading `./` and `/` strip is the step that decides a prefix match, and
//! it is why [`ArchiveService::extract_prefix`] keeps the two paths split
//! instead of normalizing uniformly. The trailing-slash strip, the
//! repeated-slash collapse and the `.` / `..` drop are Full-only too, so the
//! profiles also disagree on `Textures//X.dds`: Light keeps `textures//x.dds`,
//! Full yields `textures/x.dds`.
//!
//! ## Logging
//!
//! Every line this module writes carries the `[archive]` tag. The lines that
//! name a backend name the crate that actually ran: `zip`, `sevenz_rust2` or
//! `unrar`. Many lines name none, so `[archive]` output is not a reliable
//! source of the backend for a given archive.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::logger::Logger;
use crate::utils::{self, normalize_path, to_lower};

/// Maximum decompressed entry size buffered in memory, in bytes: 256 MiB.
///
/// A decompression-bomb guard. The in-memory read paths
/// ([`ArchiveService::read_entry`], [`ArchiveService::read_entries_batch`])
/// reject any entry whose header uncompressed size exceeds this before
/// allocating. The cap applies uniformly to all six read helpers
/// (`read_entry_zip`, `_7z`, `_rar` and `read_batch_zip`, `_7z`, `_rar`), so an
/// oversized entry comes back empty from a single read and absent from a batch
/// read, whatever the format. Archives are untrusted input; do not weaken the
/// guard or exempt a format from it. See `PARITY-NOTES.md`.
///
/// The `extract*` methods carry no such rejection, and only the ZIP path bounds
/// even the up-front allocation: `extract_zip` passes the archive-declared size
/// through `prealloc_hint`, and the buffer still grows to the real size.
/// `extract_7z` grows from an empty `Vec` and `extract_rar` takes unrar's own
/// buffer, so nothing bounds the allocation there. Every backend materializes a
/// whole entry in memory; see `COPY_CHUNK`.
pub const MAX_ENTRY_SIZE: i64 = 256 * 1024 * 1024;

/// Copy buffer for `create_zip` only: 8 KiB.
///
/// No extraction path uses it. The ZIP and 7z backends read a whole entry with
/// `read_to_end` and the RAR backend takes unrar's own buffer, so an extracted
/// entry is fully buffered in RAM rather than streamed block by block. See the
/// memory-model note in `PARITY-NOTES.md`.
const COPY_CHUNK: usize = 8192;

/// True when the entry's header uncompressed size exceeds the 256 MiB cap.
///
/// Header sizes are unsigned, so only the upper bound can trip. A forged
/// multi-gigabyte size is rejected like any other over-cap value.
fn exceeds_entry_cap(size: u64) -> bool {
    size > MAX_ENTRY_SIZE as u64
}

/// Clamp an archive-declared (untrusted) uncompressed size before using it as a
/// `Vec::with_capacity` hint.
///
/// The zip central directory records the uncompressed size as an
/// attacker-controlled `u64` (zip64), and `extract` reads whole entries into
/// memory. Passing that raw size to `with_capacity` lets a forged value force an
/// unbounded up-front allocation before a single byte is read: an uncatchable
/// `handle_alloc_error` abort below `isize::MAX`, a "capacity overflow" panic
/// beyond it. Clamping the hint to the 256 MiB cap bounds the pre-allocation,
/// and the buffer still grows to the real size as bytes stream in.
fn prealloc_hint(size: u64) -> usize {
    size.min(MAX_ENTRY_SIZE as u64) as usize
}

/// Archive format after extension routing: `.7z` and `.001` ->
/// [`Format::SevenZ`], `.rar` -> [`Format::Rar`], everything else ->
/// [`Format::Zip`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Zip,
    SevenZ,
    Rar,
}

/// Errors from the fallible archive operations: [`ArchiveService::extract`],
/// [`ArchiveService::extract_filtered`], [`ArchiveService::extract_prefix`] and
/// [`ArchiveService::create_zip`]. The listing and read methods never surface an
/// error; they return empty results on failure.
#[derive(Debug)]
pub enum ArchiveError {
    /// The archive could not be opened or a fatal read error occurred.
    Open(String),
    /// A filesystem write (create dir, write file) failed during extraction or
    /// zip creation.
    Io(std::io::Error),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArchiveError::Open(msg) => write!(f, "archive open error: {msg}"),
            ArchiveError::Io(err) => write!(f, "archive io error: {err}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<std::io::Error> for ArchiveError {
    fn from(err: std::io::Error) -> Self {
        ArchiveError::Io(err)
    }
}

/// Result alias for the fallible archive operations.
pub type ArchiveResult<T> = Result<T, ArchiveError>;

/// Result of a header-only archive scan.
///
/// `paths` preserves each entry's original casing and per-format separators in
/// listing order. `sizes` maps the [`normalize_path`] key (lowercase,
/// forward-slash) to the uncompressed size in bytes, for case-insensitive
/// lookup.
#[derive(Debug, Default)]
pub struct EntryListing {
    /// Entry paths in listing order (original casing / separators).
    pub paths: Vec<String>,
    /// Normalized path -> uncompressed size in bytes.
    pub sizes: HashMap<String, u64>,
}

/// Unified archive I/O facade. Stateless: each call opens the archive fresh.
///
/// The handle is zero-sized and `Copy`, so it is trivially `Send + Sync` and
/// costs nothing to share between threads. The only process-global state the
/// methods touch is the `Logger` singleton, whose file state is mutex-guarded.
/// The constraint is on the destination, not the handle: two extractions running
/// at the same time into the same destination directory race on the same output
/// files, so concurrent calls are safe only when their destinations differ.
#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveService;

impl ArchiveService {
    /// Construct a service handle. There is no state to build.
    pub fn new() -> Self {
        ArchiveService
    }

    /// Whether an archive routes to the 7z/rar backend: true when the lowercased
    /// final extension is `7z`, `rar` or `001`.
    pub fn use_bit7z(archive_path: &str) -> bool {
        matches!(extension_lower(archive_path).as_str(), "7z" | "rar" | "001")
    }

    /// List entries with uncompressed sizes, reading headers only.
    ///
    /// Returns an empty listing on any open or read failure; it never errors,
    /// so an unreadable archive is indistinguishable from an empty one. The
    /// module docs hold the per-format order and separator rules.
    pub fn list_entries_with_sizes(&self, archive_path: &str) -> EntryListing {
        let backend = backend_name(format_of(archive_path));
        let ext = dotted_extension(archive_path);
        Logger::instance().log(&format!(
            "[archive] list_entries via {backend} for {ext} file"
        ));

        let started = Instant::now();
        let listing = match self.list_raw(archive_path) {
            Ok(raw) => build_listing(raw),
            Err(_) => EntryListing::default(),
        };
        let ms = started.elapsed().as_millis();
        Logger::instance().log(&format!(
            "[archive] list_entries: {} entries, {} sizes via {backend} ({ms}ms)",
            listing.paths.len(),
            listing.sizes.len()
        ));
        listing
    }

    /// List entry paths only: the `paths` vector from
    /// [`Self::list_entries_with_sizes`], same order and casing.
    pub fn list_entries(&self, archive_path: &str) -> Vec<String> {
        self.list_entries_with_sizes(archive_path).paths
    }

    /// Read a single entry into memory.
    ///
    /// Matching is case-insensitive with normalized separators
    /// ([`normalize_path`], the Full profile). Returns an empty vector when the
    /// entry is missing, its header size exceeds the 256 MiB cap, or the archive
    /// cannot be opened. No error is surfaced, so the three cases look alike.
    pub fn read_entry(&self, archive_path: &str, entry_name: &str) -> Vec<u8> {
        let target = normalize_path(entry_name);
        match format_of(archive_path) {
            Format::Zip => read_entry_zip(archive_path, &target).unwrap_or_default(),
            Format::SevenZ => read_entry_7z(archive_path, &target).unwrap_or_default(),
            Format::Rar => read_entry_rar(archive_path, &target).unwrap_or_default(),
        }
    }

    /// Read several entries in a single archive pass.
    ///
    /// `entry_names` must already be Full-normalized (lowercase, forward-slash);
    /// each archive entry is normalized and looked up in that set. The returned
    /// map is keyed by the normalized path, and an entry the archive does not
    /// hold is simply absent. A solid 7z block is decoded once through
    /// `for_each_entries` rather than re-decoded per entry.
    ///
    /// Two entries can normalize onto one key, for example `Textures/x.dds` and
    /// `textures/x.dds`. Which one's bytes survive depends on the backend: zip
    /// keeps the last such entry, 7z and rar keep the first.
    /// [`Self::read_entry`] keeps the first for every format.
    pub fn read_entries_batch(
        &self,
        archive_path: &str,
        entry_names: &HashSet<String>,
    ) -> HashMap<String, Vec<u8>> {
        // This check runs before the log line, so an empty request emits nothing
        // at all and never opens the archive.
        if entry_names.is_empty() {
            return HashMap::new();
        }
        Logger::instance().log(&format!(
            "[archive] read_entries_batch: {} entries requested",
            entry_names.len()
        ));
        let format = format_of(archive_path);
        let started = Instant::now();
        let result = match format {
            Format::Zip => read_batch_zip(archive_path, entry_names),
            Format::SevenZ => read_batch_7z(archive_path, entry_names),
            Format::Rar => read_batch_rar(archive_path, entry_names),
        };
        let results = result.unwrap_or_default();
        let ms = started.elapsed().as_millis();
        Logger::instance().log(&format!(
            "[archive] read_entries_batch: {}/{} entries via {} ({ms}ms)",
            results.len(),
            entry_names.len(),
            backend_name(format)
        ));
        results
    }

    /// Extract every entry to a destination directory.
    ///
    /// An entry whose resolved path would escape `destination_path` is skipped
    /// and never written (the traversal guard, [`utils::is_inside`]). Errors if
    /// the archive cannot be opened.
    pub fn extract(&self, archive_path: &str, destination_path: &str) -> ArchiveResult<()> {
        // Calls the shared counted body rather than `extract_filtered`, whose
        // closing log line belongs to that entry point alone.
        Logger::instance().log(&format!("[archive] Extracting archive: {archive_path}"));
        let count = self.extract_counted(archive_path, destination_path, |_| true)?;
        Logger::instance().log(&format!(
            "[archive] Extraction completed via {}: {count} entries",
            backend_name(format_of(archive_path))
        ));
        Ok(())
    }

    /// Extract only the entries `filter` accepts.
    ///
    /// `filter` receives each entry's path with its original casing, but the
    /// separators are per backend:
    ///
    /// - ZIP passes the `zip` crate's stored name unchanged, so forward slashes.
    /// - RAR passes unrar's native name unchanged, so backslashes on Windows.
    /// - 7z passes the backslash form: `sevenz_rust2` reports forward slashes
    ///   and [`sevenz_backslash_name`] rewrites every `/` to `\` before the
    ///   closure sees it.
    ///
    /// A filter that must work for every format therefore has to be
    /// separator-insensitive, for example by running [`normalize_path`] on its
    /// argument before matching. `safe_output_path` builds the output path from
    /// the same string the filter saw, so accepting an entry and predicting
    /// where it lands use identical input.
    ///
    /// Rejecting an entry costs no memory for ZIP (the entry is never read) or
    /// RAR (unrar skips it), but a rejected 7z entry is still pushed through the
    /// block decoder into a sink: the solid block stays aligned only if every
    /// entry before the last kept one is drained. See `for_each_7z_entry`.
    pub fn extract_filtered<F>(
        &self,
        archive_path: &str,
        destination_path: &str,
        filter: F,
    ) -> ArchiveResult<()>
    where
        F: FnMut(&str) -> bool,
    {
        let count = self.extract_counted(archive_path, destination_path, filter)?;
        Logger::instance().log(&format!(
            "[archive] extract_filtered: extracted {count} entries"
        ));
        Ok(())
    }

    /// Route to the per-format extraction backend and return the number of
    /// entries actually written. Shared silent body behind [`Self::extract`],
    /// [`Self::extract_filtered`] and [`Self::extract_prefix`], each of which
    /// emits its own log lines.
    fn extract_counted<F>(
        &self,
        archive_path: &str,
        destination_path: &str,
        filter: F,
    ) -> ArchiveResult<usize>
    where
        F: FnMut(&str) -> bool,
    {
        match format_of(archive_path) {
            Format::Zip => extract_zip(archive_path, destination_path, filter),
            Format::SevenZ => extract_7z(archive_path, destination_path, filter),
            Format::Rar => extract_rar(archive_path, destination_path, filter),
        }
    }

    /// Extract entries whose normalized path starts with `prefix`.
    ///
    /// Comparison is case-insensitive with `\` -> `/` folding on both sides. The
    /// prefix always gets the Light treatment: lowercase and slash-fold, no
    /// stripping. The entry side is normalized per format:
    ///
    /// - ZIP uses Light too, so a stored `/textures/x` does not match prefix
    ///   `textures`.
    /// - 7z and rar use Full ([`normalize_path`]), whose four extra steps strip
    ///   a leading `./` and `/`, strip a trailing `/`, collapse repeated `/`,
    ///   and drop `.` and `..` segments.
    ///
    /// The split is deliberate. A uniform Full profile would make the ZIP path
    /// accept strictly more entries (every leading-`/` or `./` one), which
    /// changes what an install replay writes to disk. Keep the two paths apart.
    pub fn extract_prefix(
        &self,
        archive_path: &str,
        destination_path: &str,
        prefix: &str,
    ) -> ArchiveResult<()> {
        let prefix_lower = to_lower(prefix).replace('\\', "/");
        let format = format_of(archive_path);
        let count = self.extract_counted(archive_path, destination_path, |entry_path| {
            prefix_entry_norm(format, entry_path).starts_with(&prefix_lower)
        })?;
        Logger::instance().log(&format!(
            "[archive] {} extract_prefix: {count} entries",
            backend_name(format)
        ));
        Ok(())
    }

    /// Create a zip archive from a directory tree.
    ///
    /// Adds every file under `folder_path` recursively with default deflate
    /// compression; directory entries are implied, not stored. Parent
    /// directories of `output_zip_path` are created. Entry names use forward
    /// slashes, and entry order is the sorted order of the collected file paths,
    /// so the same tree always produces the same sequence. See `PARITY-NOTES.md`
    /// for the separator choice.
    ///
    /// **Failure modes.** The method errors when the output file cannot be
    /// created, when `start_file` fails for an entry, when an input file cannot
    /// be opened, when a read of an input file fails, when a write into the zip
    /// stream fails, or when `finish` fails. Each of these abandons the
    /// partially written archive rather than skipping the offending entry, so
    /// the archive can be left truncated and unreadable, and the caller must
    /// delete it. That control flow is deliberate; see `PARITY-NOTES.md` before
    /// turning any of these errors back into a skip.
    ///
    /// Two cases are dropped instead of surfaced: a subdirectory whose
    /// `read_dir` fails, and a collected path that does not start with
    /// `folder_path`. Both skip silently, without a warning or an error, so an
    /// unreadable subtree yields a smaller archive and `Ok(())`.
    ///
    /// No engine code calls this. Only unit tests and the zip fixture builder in
    /// the `capi` tests exercise it.
    pub fn create_zip(&self, folder_path: &str, output_zip_path: &str) -> ArchiveResult<()> {
        create_zip_impl(folder_path, output_zip_path)
    }

    /// Collect the raw (original-casing path, size) list for a format, applying
    /// the per-format directory-strip / separator / ordering rules. Errors only
    /// on open failure; the public listing methods map that to an empty result.
    fn list_raw(&self, archive_path: &str) -> ArchiveResult<Vec<(String, u64)>> {
        match format_of(archive_path) {
            Format::Zip => list_zip(archive_path),
            Format::SevenZ => list_7z(archive_path),
            Format::Rar => list_rar(archive_path),
        }
    }
}

/// Lowercased final extension of a path, without the dot, from
/// [`Path::extension`].
///
/// That is not "the text after the last `.`". [`Path::extension`] yields nothing
/// when the file name holds no embedded dot, or when it begins with a dot and
/// has no other dot, so `.7z` and `C:/a/.7z` both give `""` and `format_of`
/// routes them to [`Format::Zip`]. `a.7z` gives `7z`, `mod.tar.gz` gives `gz`,
/// `a.` gives `""`.
fn extension_lower(archive_path: &str) -> String {
    Path::new(archive_path)
        .extension()
        .map(|e| to_lower(&e.to_string_lossy()))
        .unwrap_or_default()
}

/// Route an archive path to its backend format by extension.
fn format_of(archive_path: &str) -> Format {
    match extension_lower(archive_path).as_str() {
        "rar" => Format::Rar,
        // `.001` routes to the 7z backend. The extension mapping is covered by
        // `format_routing` and `use_bit7z_matches_extension_set`; opening a
        // genuine split volume set is not, and that is a known behavioral gap
        // rather than a coverage gap. sevenz_rust2 0.21.3 has no multi-volume
        // support and both `Archive::open` and `ArchiveReader::open` take a
        // single path, so a real `.001` first volume, whose end-of-archive
        // header lives in the last volume, is expected to fail to open.
        "7z" | "001" => Format::SevenZ,
        _ => Format::Zip,
    }
}

/// Normalize an entry path for [`ArchiveService::extract_prefix`] matching, per
/// format.
///
/// ZIP uses the Light profile, exactly `to_lower(entry).replace('\\', "/")`. 7z
/// and rar use Full ([`normalize_path`]), which adds four steps on top of Light:
/// strip a leading `./` and `/`, strip a trailing `/`, collapse repeated `/`,
/// drop `.` and `..` segments. The leading strip is the step that changes prefix
/// results, so `./textures/x.dds` matches prefix `textures` for 7z and rar but
/// not for ZIP. The module-level normalization section traces one entry through
/// both profiles.
fn prefix_entry_norm(format: Format, entry_path: &str) -> String {
    match format {
        Format::Zip => to_lower(entry_path).replace('\\', "/"),
        Format::SevenZ | Format::Rar => normalize_path(entry_path),
    }
}

/// Build an [`EntryListing`] from the per-format ordered raw list. `paths` keeps
/// the original strings in order; `sizes` maps `normalize_path(path)` to the
/// uncompressed size in bytes.
///
/// Two entries can collapse onto one `sizes` key, for example `Textures/x.dds`
/// and `textures/x.dds` in a case-preserving archive. The last such entry in
/// listing order wins, because `HashMap::insert` overwrites. Both entries still
/// appear in `paths`, so `paths.len()` can exceed `sizes.len()`.
fn build_listing(raw: Vec<(String, u64)>) -> EntryListing {
    let mut listing = EntryListing::default();
    for (path, size) in raw {
        listing.sizes.insert(normalize_path(&path), size);
        listing.paths.push(path);
    }
    listing
}

/// Stable case-insensitive sort of (path, size) pairs by the backslash path, the
/// ordering step for 7z and rar. Stable, so entries equal under lowercasing keep
/// their raw header order.
fn ci_sort_by_path(entries: &mut [(String, u64)]) {
    entries.sort_by_key(|entry| to_lower(&entry.0));
}

/// Join an entry's (possibly hostile) path onto the destination and confirm the
/// result stays inside it.
///
/// Returns the safe output path, or `None` when the entry must be skipped.
/// `None` has two causes and the caller cannot tell them apart:
///
/// 1. A genuine traversal: the joined path resolves outside `destination`.
/// 2. A canonicalization failure on either path for any reason other than
///    not-found, for example a permission or I/O error. [`utils::is_inside`]
///    deliberately treats that as false.
///
/// Both cases log the same `[archive] Skipping path-traversal entry` warning, so
/// an environment failure is reported with traversal wording, and the extraction
/// quietly produces fewer files instead of returning an error. The guard fails
/// closed on purpose; do not change that.
fn safe_output_path(destination: &Path, entry_path: &str) -> Option<PathBuf> {
    let full_output = destination.join(entry_path);
    if utils::is_inside(destination, &full_output) {
        Some(full_output)
    } else {
        // All three extraction backends route through this guard, so the warning
        // has a single call site.
        Logger::instance().log_warning(&format!(
            "[archive] Skipping path-traversal entry: {entry_path}"
        ));
        None
    }
}

/// The extension with its leading dot, lowercased, as the `[archive]` log lines
/// carry it: "for .7z file". An extensionless path yields "", so the line reads
/// "for  file" rather than showing a bare ".".
fn dotted_extension(archive_path: &str) -> String {
    let ext = extension_lower(archive_path);
    if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    }
}

/// Name of the crate that backs a format, for the `[archive]` log lines.
fn backend_name(format: Format) -> &'static str {
    match format {
        Format::Zip => "zip",
        Format::SevenZ => "sevenz_rust2",
        Format::Rar => "unrar",
    }
}

/// Count an extracted entry and emit the progress line every 100 entries.
fn note_extracted(count: &mut usize) {
    *count += 1;
    if *count % 100 == 0 {
        Logger::instance().log(&format!("[archive] Extracted {count} files..."));
    }
}

/// Write a file's bytes to `output`, creating parent directories first. Shared
/// by every extraction backend.
fn write_extracted_file(output: &Path, bytes: &[u8]) -> ArchiveResult<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(output)?;
    file.write_all(bytes)?;
    Ok(())
}

/// List a zip in native central-directory order, skipping directory entries and
/// keeping the stored forward-slash names.
///
/// Skipping `is_dir()` entries is required, not incidental. A zip may store
/// explicit directory markers, and the listing must contain files only.
/// Restoring the markers changes the entry count and the entry order for every
/// archive that stores them, which shifts what later stages see. The shape test
/// `list_zip_strips_dirs_keeps_order_and_sizes` below is the only in-repo check;
/// `PARITY-NOTES.md` records the measurement that fixed the rule.
fn list_zip(archive_path: &str) -> ArchiveResult<Vec<(String, u64)>> {
    let file = File::open(archive_path).map_err(|e| ArchiveError::Open(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| ArchiveError::Open(e.to_string()))?;
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| ArchiveError::Open(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        out.push((entry.name().to_string(), entry.size()));
    }
    Ok(out)
}

fn read_entry_zip(archive_path: &str, target: &str) -> Option<Vec<u8>> {
    let file = File::open(archive_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).ok()?;
        if entry.is_dir() {
            continue;
        }
        if normalize_path(entry.name()) != target {
            continue;
        }
        if exceeds_entry_cap(entry.size()) {
            return Some(Vec::new());
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).ok()?;
        return Some(buf);
    }
    Some(Vec::new())
}

fn read_batch_zip(
    archive_path: &str,
    entry_names: &HashSet<String>,
) -> Option<HashMap<String, Vec<u8>>> {
    let file = File::open(archive_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut results = HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).ok()?;
        if entry.is_dir() {
            continue;
        }
        let norm = normalize_path(entry.name());
        if !entry_names.contains(&norm) {
            continue;
        }
        if exceeds_entry_cap(entry.size()) {
            continue;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        if entry.read_to_end(&mut buf).is_ok() {
            results.insert(norm, buf);
        }
    }
    Some(results)
}

fn extract_zip<F>(archive_path: &str, destination: &str, mut filter: F) -> ArchiveResult<usize>
where
    F: FnMut(&str) -> bool,
{
    let mut count = 0usize;
    let dest = Path::new(destination);
    fs::create_dir_all(dest)?;
    let file = File::open(archive_path).map_err(|e| ArchiveError::Open(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| ArchiveError::Open(e.to_string()))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| ArchiveError::Open(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !filter(&name) {
            continue;
        }
        let Some(output) = safe_output_path(dest, &name) else {
            continue;
        };
        // Clamp the capacity hint: `entry.size()` is the archive-controlled
        // uncompressed size, so a forged value would otherwise abort or panic on
        // the pre-allocation. `extract` deliberately applies no 256 MiB
        // rejection here; the buffer still grows to the real size via
        // `read_to_end`.
        let mut buf = Vec::with_capacity(prealloc_hint(entry.size()));
        entry.read_to_end(&mut buf)?;
        write_extracted_file(&output, &buf)?;
        note_extracted(&mut count);
    }
    Ok(count)
}

/// Rewrite a 7z entry name into the backslash form the listing rules require.
/// `sevenz_rust2` reports forward slashes, so every `/` is converted.
fn sevenz_backslash_name(name: &str) -> String {
    name.replace('/', "\\")
}

/// List a 7z: header-only open, skip directories, `\`-form names,
/// case-insensitive sort by the backslash path.
fn list_7z(archive_path: &str) -> ArchiveResult<Vec<(String, u64)>> {
    let archive =
        sevenz_rust2::Archive::open(archive_path).map_err(|e| ArchiveError::Open(e.to_string()))?;
    let mut out = Vec::with_capacity(archive.files.len());
    for entry in &archive.files {
        if entry.is_directory() {
            continue;
        }
        out.push((sevenz_backslash_name(entry.name()), entry.size()));
    }
    ci_sort_by_path(&mut out);
    Ok(out)
}

fn read_entry_7z(archive_path: &str, target: &str) -> Option<Vec<u8>> {
    let mut found: Option<Vec<u8>> = None;
    for_each_7z_entry(archive_path, |name, size, read| {
        if found.is_some() {
            return Ok(true);
        }
        if normalize_path(name) == target {
            if exceeds_entry_cap(size) {
                found = Some(Vec::new());
                return Ok(false);
            }
            let mut buf = Vec::with_capacity(size as usize);
            read.read_to_end(&mut buf)?;
            found = Some(buf);
            return Ok(false);
        }
        Ok(true)
    })
    .ok()?;
    // Match not found -> empty (never None-on-not-found; only None on open err).
    Some(found.unwrap_or_default())
}

fn read_batch_7z(
    archive_path: &str,
    entry_names: &HashSet<String>,
) -> Option<HashMap<String, Vec<u8>>> {
    let mut results = HashMap::new();
    for_each_7z_entry(archive_path, |name, size, read| {
        let norm = normalize_path(name);
        if entry_names.contains(&norm) && !results.contains_key(&norm) && !exceeds_entry_cap(size) {
            let mut buf = Vec::with_capacity(size as usize);
            read.read_to_end(&mut buf)?;
            results.insert(norm, buf);
        }
        // Stop once every requested entry has been collected. Unmatched or
        // over-cap entries, and every entry before the last match, keep the pass
        // going; `for_each_7z_entry` drains each one so the solid block stays
        // aligned.
        Ok(results.len() < entry_names.len())
    })
    .ok()?;
    Some(results)
}

/// Decode a 7z once, invoking `each(name, size, reader)` per file entry. The
/// closure returns `Ok(true)` to continue or `Ok(false)` to stop early. This is
/// the solid-block-friendly path: `for_each_entries` decodes each block a single
/// time and streams every entry in it. `name` carries the crate's forward-slash
/// spelling; callers that need the backslash form convert with
/// [`sevenz_backslash_name`].
///
/// **Solid-block alignment.** Every entry in a solid block reads from one shared
/// decode stream, and that stream advances only by the bytes the closure
/// actually consumes. An entry the closure ignores must therefore still be
/// drained, which this function does after the closure returns `Ok(true)`:
///
/// ```text
/// solid block: one decode stream, cursor moves only by bytes that are read
///
///   [--- a.dds ---][--- b.esp ---][--- c.nif ---]
///   ^cursor
///
/// good  skipped entry drained into a sink:
///   [=== a.dds ===][--- b.esp ---]...
///                  ^cursor    b.esp decodes correctly
///
/// bad   skipped entry left partly read:
///   [== a.dds ==...][--- b.esp ---]...
///               ^cursor    b.esp decodes from inside a.dds:
///                          wrong bytes, or a Crc32VerifyingReader error
///                          that surfaces as empty, absent or Err
/// ```
///
/// The drain is not dead work. Do not remove it.
fn for_each_7z_entry<F>(archive_path: &str, mut each: F) -> Result<(), sevenz_rust2::Error>
where
    F: FnMut(&str, u64, &mut dyn Read) -> Result<bool, std::io::Error>,
{
    let mut reader =
        sevenz_rust2::ArchiveReader::open(archive_path, sevenz_rust2::Password::empty())?;
    reader.for_each_entries(|entry, rd| {
        let keep_going = if entry.is_directory() {
            true
        } else {
            let name = entry.name().to_string();
            let size = entry.size();
            each(&name, size, rd).map_err(sevenz_rust2::Error::from)?
        };
        if keep_going {
            // Solid-block alignment: fully drain this entry's reader before the
            // block decoder advances to the next file. The diagram in this
            // function's doc comment shows what a partial read does to the next
            // entry. sevenz_rust2 layers every file's reader (a `BoundedReader`,
            // optionally wrapped in a `Crc32VerifyingReader`) over one shared
            // per-block decode stream and gives it no Drop or auto-skip
            // (reader.rs), so the shared stream moves forward only by bytes the
            // closure actually read. The closure reads nothing in the
            // not-matching, filtered-out and traversal cases, so those are
            // exactly the entries that need this drain. The crate's own
            // `read_file` does the same thing for the same reason. Draining is
            // skipped only when stopping early (`keep_going == false`), where no
            // further entry is decoded.
            std::io::copy(rd, &mut std::io::sink()).map_err(sevenz_rust2::Error::from)?;
        }
        Ok(keep_going)
    })
}

fn extract_7z<F>(archive_path: &str, destination: &str, mut filter: F) -> ArchiveResult<usize>
where
    F: FnMut(&str) -> bool,
{
    let mut count = 0usize;
    let dest = Path::new(destination);
    fs::create_dir_all(dest)?;
    let mut io_err: Option<std::io::Error> = None;
    for_each_7z_entry(archive_path, |name, _size, read| {
        // The filter contract gives 7z entries backslash separators.
        let back = sevenz_backslash_name(name);
        if !filter(&back) {
            return Ok(true);
        }
        let Some(output) = safe_output_path(dest, &back) else {
            return Ok(true);
        };
        let mut buf = Vec::new();
        read.read_to_end(&mut buf)?;
        if let Err(e) = write_extracted_file(&output, &buf) {
            // Surface the first write error after the decode loop unwinds.
            io_err = Some(match e {
                ArchiveError::Io(io) => io,
                ArchiveError::Open(msg) => std::io::Error::other(msg),
            });
            return Ok(false);
        }
        note_extracted(&mut count);
        Ok(true)
    })
    .map_err(|e| ArchiveError::Open(e.to_string()))?;
    if let Some(e) = io_err {
        return Err(ArchiveError::Io(e));
    }
    Ok(count)
}

/// unrar reports the filename as a `PathBuf` built from the RAR wide-char name,
/// which preserves the archive's native backslash separators on Windows.
fn rar_name(header: &unrar::FileHeader) -> String {
    header.filename.to_string_lossy().into_owned()
}

/// List a RAR: iterate headers in listing mode, skip directories, keep unrar's
/// native `\` names, case-insensitive sort by the backslash path.
fn list_rar(archive_path: &str) -> ArchiveResult<Vec<(String, u64)>> {
    let archive = unrar::Archive::new(archive_path)
        .open_for_listing()
        .map_err(|e| ArchiveError::Open(e.to_string()))?;
    let mut out = Vec::new();
    for header in archive {
        let header = header.map_err(|e| ArchiveError::Open(e.to_string()))?;
        if header.is_directory() {
            continue;
        }
        out.push((rar_name(&header), header.unpacked_size));
    }
    ci_sort_by_path(&mut out);
    Ok(out)
}

fn read_entry_rar(archive_path: &str, target: &str) -> Option<Vec<u8>> {
    let mut archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .ok()?;
    while let Some(open) = archive.read_header().ok()? {
        let header = open.entry();
        if header.is_directory() {
            archive = open.skip().ok()?;
            continue;
        }
        if normalize_path(&rar_name(header)) == target {
            if exceeds_entry_cap(header.unpacked_size) {
                return Some(Vec::new());
            }
            let (bytes, _next) = open.read().ok()?;
            return Some(bytes);
        }
        archive = open.skip().ok()?;
    }
    Some(Vec::new())
}

fn read_batch_rar(
    archive_path: &str,
    entry_names: &HashSet<String>,
) -> Option<HashMap<String, Vec<u8>>> {
    let mut archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .ok()?;
    let mut results = HashMap::new();
    let mut remaining = entry_names.len();
    while remaining > 0 {
        let Some(open) = archive.read_header().ok()? else {
            break;
        };
        let header = open.entry();
        let norm = normalize_path(&rar_name(header));
        if header.is_directory() || !entry_names.contains(&norm) || results.contains_key(&norm) {
            archive = open.skip().ok()?;
            continue;
        }
        if exceeds_entry_cap(header.unpacked_size) {
            archive = open.skip().ok()?;
            continue;
        }
        let (bytes, next) = open.read().ok()?;
        results.insert(norm, bytes);
        remaining -= 1;
        archive = next;
    }
    Some(results)
}

fn extract_rar<F>(archive_path: &str, destination: &str, mut filter: F) -> ArchiveResult<usize>
where
    F: FnMut(&str) -> bool,
{
    let mut count = 0usize;
    let dest = Path::new(destination);
    fs::create_dir_all(dest)?;
    let mut archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .map_err(|e| ArchiveError::Open(e.to_string()))?;
    loop {
        let Some(open) = archive
            .read_header()
            .map_err(|e| ArchiveError::Open(e.to_string()))?
        else {
            break;
        };
        let header = open.entry();
        let name = rar_name(header);
        if header.is_directory() || !filter(&name) {
            archive = open.skip().map_err(|e| ArchiveError::Open(e.to_string()))?;
            continue;
        }
        let output = safe_output_path(dest, &name);
        match output {
            Some(output) => {
                let (bytes, next) = open.read().map_err(|e| ArchiveError::Open(e.to_string()))?;
                write_extracted_file(&output, &bytes)?;
                note_extracted(&mut count);
                archive = next;
            }
            None => {
                archive = open.skip().map_err(|e| ArchiveError::Open(e.to_string()))?;
            }
        }
    }
    Ok(count)
}

fn create_zip_impl(folder_path: &str, output_zip_path: &str) -> ArchiveResult<()> {
    let out_path = Path::new(output_zip_path);
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let out_file = File::create(out_path).map_err(|e| ArchiveError::Open(e.to_string()))?;
    let mut writer = zip::ZipWriter::new(out_file);
    let options: zip::write::FileOptions<'_, ()> = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let folder = Path::new(folder_path);
    let mut stack = vec![folder.to_path_buf()];
    // Collect every file into one flat vector first. `fs::read_dir` order is
    // unspecified, so the walk alone is not deterministic.
    let mut files: Vec<PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        let mut children: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => continue,
        };
        // Only orders the stack pushes; the global sort below overrides it.
        children.sort();
        for child in children {
            if child.is_dir() {
                stack.push(child);
            } else {
                files.push(child);
            }
        }
    }
    // This sort is what fixes the zip entry order: entries are written in
    // `files` order, so the same tree always yields the same archive layout.
    files.sort();

    for path in files {
        let Ok(rel) = path.strip_prefix(folder) else {
            continue;
        };
        // zip entry names use forward slashes regardless of host separator.
        let rel_name = rel.to_string_lossy().replace('\\', "/");
        writer
            .start_file(rel_name.clone(), options)
            .map_err(|e| ArchiveError::Open(e.to_string()))?;
        // The warning says "skipping", but the `?` propagates and abandons the
        // partially written archive. That mismatch is deliberate; see the
        // failure-modes note on `create_zip` and `PARITY-NOTES.md` before
        // turning it into a `continue`.
        let mut input = File::open(&path).inspect_err(|_| {
            Logger::instance().log_warning(&format!(
                "[archive] Skipping file in zip (cannot read size): {rel_name}"
            ));
        })?;
        let mut chunk = [0u8; COPY_CHUNK];
        loop {
            let n = input.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            // Same shape as the open above: warn, then propagate and abandon the
            // archive.
            writer.write_all(&chunk[..n]).map_err(|e| {
                Logger::instance()
                    .log_warning(&format!("[archive] Write error in zip for: {rel_name}"));
                ArchiveError::Io(std::io::Error::other(e.to_string()))
            })?;
        }
    }
    writer
        .finish()
        .map_err(|e| ArchiveError::Open(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // --- extension routing / use_bit7z ---

    #[test]
    fn use_bit7z_matches_extension_set() {
        assert!(ArchiveService::use_bit7z("mod.7z"));
        assert!(ArchiveService::use_bit7z("mod.RAR"));
        assert!(ArchiveService::use_bit7z("mod.001"));
        assert!(ArchiveService::use_bit7z("C:/a/b/Mod.7Z"));
        assert!(!ArchiveService::use_bit7z("mod.zip"));
        assert!(!ArchiveService::use_bit7z("mod.tar.gz"));
        assert!(!ArchiveService::use_bit7z("mod"));
    }

    #[test]
    fn format_routing() {
        assert_eq!(format_of("a.zip"), Format::Zip);
        assert_eq!(format_of("a.7z"), Format::SevenZ);
        assert_eq!(format_of("a.001"), Format::SevenZ);
        assert_eq!(format_of("a.rar"), Format::Rar);
        assert_eq!(format_of("a.tar.gz"), Format::Zip);
    }

    // --- 256 MiB cap ---

    #[test]
    fn entry_cap_constant_is_256_mib() {
        assert_eq!(MAX_ENTRY_SIZE, 256 * 1024 * 1024);
        assert_eq!(MAX_ENTRY_SIZE, 268_435_456);
    }

    #[test]
    fn entry_cap_guard_boundary() {
        // At the cap: allowed. One over: rejected. Forged huge: rejected.
        assert!(!exceeds_entry_cap(MAX_ENTRY_SIZE as u64));
        assert!(!exceeds_entry_cap(0));
        assert!(exceeds_entry_cap(MAX_ENTRY_SIZE as u64 + 1));
        assert!(exceeds_entry_cap(u64::MAX));
    }

    #[test]
    fn prealloc_hint_clamps_to_cap() {
        // Under the cap: passed through unchanged as the capacity hint.
        assert_eq!(prealloc_hint(0), 0);
        assert_eq!(prealloc_hint(1024), 1024);
        assert_eq!(
            prealloc_hint(MAX_ENTRY_SIZE as u64),
            MAX_ENTRY_SIZE as usize
        );
        // Over the cap - a forged multi-gigabyte or u64::MAX uncompressed size
        // cannot force an unbounded pre-allocation; the hint saturates at the cap
        // (the buffer still grows to the real size via read_to_end).
        assert_eq!(
            prealloc_hint(MAX_ENTRY_SIZE as u64 + 1),
            MAX_ENTRY_SIZE as usize
        );
        assert_eq!(
            prealloc_hint(8 * 1024 * 1024 * 1024),
            MAX_ENTRY_SIZE as usize
        );
        assert_eq!(prealloc_hint(u64::MAX), MAX_ENTRY_SIZE as usize);
    }

    #[test]
    fn extract_prefix_entry_norm_light_for_zip_full_for_7z_rar() {
        // ZIP uses Light: lowercase and slash-fold, no leading-./ or -/ strip,
        // so a stored "./x" or "/x" keeps its leading segment.
        assert_eq!(
            prefix_entry_norm(Format::Zip, "./Textures/X.dds"),
            "./textures/x.dds"
        );
        assert_eq!(
            prefix_entry_norm(Format::Zip, "/Textures/X.dds"),
            "/textures/x.dds"
        );
        assert_eq!(
            prefix_entry_norm(Format::Zip, "Textures\\X.dds"),
            "textures/x.dds"
        );
        // 7z and rar use Full normalize_path, which strips leading ./ and /.
        assert_eq!(
            prefix_entry_norm(Format::SevenZ, "./Textures/X.dds"),
            "textures/x.dds"
        );
        assert_eq!(
            prefix_entry_norm(Format::Rar, "\\Textures\\X.dds"),
            "textures/x.dds"
        );
        // The observable difference this pins: prefix "textures" matches a
        // leading-"./" entry under the 7z Full profile but not under zip Light.
        let leading = "./textures/x.dds";
        assert!(!prefix_entry_norm(Format::Zip, leading).starts_with("textures"));
        assert!(prefix_entry_norm(Format::SevenZ, leading).starts_with("textures"));
    }

    // --- in-memory zip builders (no external files) ---

    /// Build an in-memory zip from (name, bytes) pairs, in the given order.
    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, data) in entries {
                w.start_file(*name, opts).expect("start_file");
                w.write_all(data).expect("write");
            }
            w.finish().expect("finish");
        }
        buf
    }

    /// Build an in-memory zip that also stores an explicit directory entry.
    fn build_zip_with_dir(dir: &str, entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            w.add_directory(dir, opts).expect("add_directory");
            for (name, data) in entries {
                w.start_file(*name, opts).expect("start_file");
                w.write_all(data).expect("write");
            }
            w.finish().expect("finish");
        }
        buf
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "salma_rs_archive_{}_{}",
            tag,
            crate::utils::random_hex_string(12)
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    // --- listing (dir stripped, stored order, sizes) ---

    #[test]
    fn list_zip_strips_dirs_keeps_order_and_sizes() {
        let dir = temp_dir("list");
        let zip_path = dir.join("t.zip");
        let bytes = build_zip_with_dir(
            "sub/",
            &[
                ("a.txt", b"hello"),
                ("sub/b.bin", b"\x00\x01\x02\x03"),
                ("c.dat", b""),
            ],
        );
        fs::write(&zip_path, &bytes).expect("write zip");

        let svc = ArchiveService::new();
        let listing = svc.list_entries_with_sizes(zip_path.to_str().unwrap());

        // The directory entry "sub/" is stripped; files keep stored order. This
        // checks the shape only. Nothing in this repository can prove the order
        // rule itself; see `list_zip` and `PARITY-NOTES.md`.
        assert_eq!(listing.paths, vec!["a.txt", "sub/b.bin", "c.dat"]);
        assert_eq!(listing.sizes["a.txt"], 5);
        assert_eq!(listing.sizes["sub/b.bin"], 4);
        assert_eq!(listing.sizes["c.dat"], 0);
        // list_entries mirrors paths.
        assert_eq!(svc.list_entries(zip_path.to_str().unwrap()), listing.paths);

        fs::remove_dir_all(&dir).ok();
    }

    // --- read_entry / read_entries_batch ---

    #[test]
    fn read_entry_case_insensitive_and_missing() {
        let dir = temp_dir("read");
        let zip_path = dir.join("t.zip");
        let bytes = build_zip(&[
            ("Fomod/ModuleConfig.xml", b"<config/>"),
            ("data/file.esp", b"ESP-BYTES"),
        ]);
        fs::write(&zip_path, &bytes).expect("write zip");
        let path = zip_path.to_str().unwrap();
        let svc = ArchiveService::new();

        // Case-insensitive, separator-normalized match.
        assert_eq!(
            svc.read_entry(path, "fomod/moduleconfig.xml"),
            b"<config/>".to_vec()
        );
        assert_eq!(
            svc.read_entry(path, "FOMOD\\MODULECONFIG.XML"),
            b"<config/>".to_vec()
        );
        // Missing entry -> empty (not an error).
        assert!(svc.read_entry(path, "nope/missing.txt").is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_entries_batch_returns_requested_subset() {
        let dir = temp_dir("batch");
        let zip_path = dir.join("t.zip");
        let bytes = build_zip(&[("a.txt", b"AAA"), ("b.txt", b"BBBB"), ("c.txt", b"CC")]);
        fs::write(&zip_path, &bytes).expect("write zip");
        let path = zip_path.to_str().unwrap();
        let svc = ArchiveService::new();

        let mut want = HashSet::new();
        want.insert("a.txt".to_string());
        want.insert("c.txt".to_string());
        want.insert("absent.txt".to_string());
        let got = svc.read_entries_batch(path, &want);

        assert_eq!(got.len(), 2);
        assert_eq!(got["a.txt"], b"AAA".to_vec());
        assert_eq!(got["c.txt"], b"CC".to_vec());
        assert!(!got.contains_key("b.txt"));
        assert!(!got.contains_key("absent.txt"));

        // Empty request set -> empty map, no archive open.
        assert!(svc.read_entries_batch(path, &HashSet::new()).is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    // --- traversal attack ---

    #[test]
    fn extract_rejects_path_traversal_entries() {
        let dir = temp_dir("traversal");
        let zip_path = dir.join("evil.zip");
        // Benign file plus malicious names that try to escape the destination:
        // relative `..` (both separators), a rooted or absolute name, and a
        // sibling-prefix escape (`../out-evil/*` from dest `out`, which shares a
        // name prefix with the destination but is not inside it).
        let bytes = build_zip(&[
            ("safe/benign.txt", b"OK"),
            ("../evil_rel.txt", b"PWNED"),
            ("..\\evil_bs.txt", b"PWNED"),
            ("/evil_abs.txt", b"PWNED"),
            ("../out-evil/sibling.txt", b"PWNED"),
        ]);
        fs::write(&zip_path, &bytes).expect("write zip");

        let out = dir.join("out");
        let svc = ArchiveService::new();
        svc.extract(zip_path.to_str().unwrap(), out.to_str().unwrap())
            .expect("extract");

        // Benign file landed inside the destination.
        assert!(out.join("safe/benign.txt").exists());
        // No escaped file landed where the escapes actually target. Assert at the
        // resolved paths, not at `out.join("evil_abs.txt")`: nothing ever writes
        // there, so such an assertion passes vacuously. The drive-root escape is
        // covered deterministically by
        // `safe_output_path_rejects_all_escape_classes` below.
        assert!(!dir.join("evil_rel.txt").exists());
        assert!(!dir.join("evil_bs.txt").exists());
        assert!(!dir.join("out-evil").exists());
        assert!(!dir.join("out-evil/sibling.txt").exists());
        // The destination tree contains exactly the benign file.
        let found = walk(&out);
        assert_eq!(found.len(), 1, "unexpected extracted files: {found:?}");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn safe_output_path_rejects_all_escape_classes() {
        // Deterministic, side-effect-free proof that the traversal guard rejects
        // every escape class, including the absolute drive-root case the
        // extraction test cannot reliably observe (a weakened guard would write
        // to C:\evil_abs.txt, invisible to a walk of the destination tree).
        let dir = temp_dir("safeout");
        let dest = dir.join("out");
        fs::create_dir_all(&dest).expect("mkdir dest");

        // Benign relative paths resolve inside the destination.
        assert!(safe_output_path(&dest, "a/b.txt").is_some());
        assert!(safe_output_path(&dest, "deep/nested/c.dat").is_some());
        // Parent-directory traversal, both separators.
        assert!(safe_output_path(&dest, "../evil_rel.txt").is_none());
        assert!(safe_output_path(&dest, "..\\evil_bs.txt").is_none());
        // Rooted-but-driveless names: on Windows dest.join("/x") replaces
        // everything after the drive prefix, giving C:\x at the drive root,
        // outside the destination. A weakened guard would write there unnoticed.
        assert!(safe_output_path(&dest, "/evil_abs.txt").is_none());
        assert!(safe_output_path(&dest, "\\evil_abs.txt").is_none());
        // Sibling-prefix escape: dest ".../out", entry resolves to ".../out-evil/x".
        assert!(safe_output_path(&dest, "../out-evil/payload.txt").is_none());

        fs::remove_dir_all(&dir).ok();
    }

    /// Recursively collect regular-file paths under `root`.
    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    // --- create_zip round-trip ---

    #[test]
    fn create_zip_then_list_round_trips() {
        let dir = temp_dir("createzip");
        let src = dir.join("src");
        fs::create_dir_all(src.join("nested")).expect("mkdir");
        fs::write(src.join("top.txt"), b"top").expect("write");
        fs::write(src.join("nested/deep.bin"), b"\x01\x02").expect("write");
        let zip_path = dir.join("out/archive.zip");

        let svc = ArchiveService::new();
        svc.create_zip(src.to_str().unwrap(), zip_path.to_str().unwrap())
            .expect("create_zip");
        assert!(zip_path.exists());

        let listing = svc.list_entries_with_sizes(zip_path.to_str().unwrap());
        let mut names = listing.paths.clone();
        names.sort();
        assert_eq!(names, vec!["nested/deep.bin", "top.txt"]);
        assert_eq!(listing.sizes["top.txt"], 3);
        assert_eq!(listing.sizes["nested/deep.bin"], 2);
        // Round-trip the content too.
        assert_eq!(
            svc.read_entry(zip_path.to_str().unwrap(), "nested/deep.bin"),
            vec![1u8, 2u8]
        );

        fs::remove_dir_all(&dir).ok();
    }
}
