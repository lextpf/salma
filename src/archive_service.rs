//! Archive I/O facade - Rust port of `src/ArchiveService.hpp` / `.cpp`.
//!
//! The C++ service fronts two libraries (libarchive for zip/tar, bit7z for
//! 7z/rar/.001). The Rust port replaces both with a pure-crate stack chosen by
//! an empirical corpus evaluation (16/16 real archives byte-match on
//! `list_entries_with_sizes`):
//!
//! - zip (`zip`)  -> ZIP: central-directory order, forward-slash paths.
//! - 7z (`sevenz_rust2`) -> 7z and `.001`: solid-block aware, header-only list.
//! - RAR (`unrar`) -> RAR: sequential; links the proprietary unRAR C sources.
//!
//! ## Byte-parity of `list_entries_with_sizes`
//!
//! The golden `archive_entries.json` snapshots the C++
//! `list_entries_with_sizes` (which routes 7z/rar through bit7z, echoing
//! 7-Zip's case-insensitive stored order). Reproducing it is per-format:
//!
//! - ZIP: native central-directory order, no sort, keep `/`, skip directory
//!   entries. (The C++ ZIP lister is libarchive, which pushes every header, but
//!   its output for the corpus zips carrying explicit directory markers contains
//!   NONE of them - libarchive does not surface those markers - so skipping
//!   `is_dir()` is the faithful match. See PARITY-NOTES "Task 11".)
//! - 7z: skip directories, convert `/` -> `\` (bit7z reports `\` for 7z), then
//!   case-insensitive sort by the backslash path.
//! - RAR: skip directories, keep unrar's native `\`, case-insensitive sort by
//!   the backslash path.
//!
//! The ci-sort is the order-parity assumption: 7-Zip/WinRAR store entries
//! case-insensitively sorted, so bit7z (and 7z.exe, which generated the golden)
//! echo that order, while `sevenz_rust2`/`unrar` return raw header order; the
//! sort reconciles them. An archive authored with an unsorted directory would
//! diverge. See `PARITY-NOTES.md` ("Task 11").
//!
//! ## Normalization profiles (from the C++ hpp table)
//!
//! `sizes` keys and the `read_entry` / `read_entries_batch` match comparisons
//! use the Full profile ([`normalize_path`]: lowercase + `\`->`/` + strip
//! leading `./` and `/`). The hpp doc table claims a "Light" profile for the
//! libarchive `read_entry` / `read_entries_batch` paths, but the C++
//! implementation calls `normalize_entry_path` (== `normalize_path`, Full)
//! uniformly on those; this port matches the implementation, not the stale doc
//! table (noted in PARITY-NOTES).
//!
//! ## `[archive]` log lines
//!
//! The C++ names its libraries in these lines ("via bit7z", "falling back to
//! libarchive", "Using libarchive for extraction"). Neither library exists here,
//! so every such line names the crate that actually ran instead - `zip`,
//! `sevenz_rust2` or `unrar` - keeping the C++ line shape, tag and position.
//! Lines describing machinery with no counterpart at all are not emitted: the
//! `7z.dll` discovery narrative ("[archive] 7z library: ...", "[archive] 7z.dll
//! not found in SEVENZIP_PATH...") and libarchive's per-entry "Write header
//! warning" / "Copy data warning" / "copy_data failed for entry" (this port has
//! no separate write-disk handle that can warn without failing). There is
//! likewise no bit7z-versus-libarchive fallback, so the four "falling back to
//! libarchive" lines have no trigger. See PARITY-NOTES "Task 17".
//!
//! [`ArchiveService::extract_prefix`] is the one deliberate exception: the C++
//! ZIP path (libarchive fallback, `ArchiveService.cpp:615-617`) normalizes the
//! entry with Light (lowercase + `\`->`/`, NO leading strip) while its bit7z
//! 7z/rar path uses Full (`:584`). The port normalizes the entry side per
//! format there (Light for ZIP, Full for 7z/rar) rather than uniformly, so a
//! stored `/textures/x` matches prefix `textures` only where the C++ would.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::logger::Logger;
use crate::utils::{self, normalize_path, to_lower};

/// Maximum decompressed entry size buffered in memory (256 MiB), mirror of
/// `kMaxEntrySize` in `ArchiveService.cpp`. A decompression-bomb guard: the
/// in-memory read paths ([`ArchiveService::read_entry`],
/// [`ArchiveService::read_entries_batch`]) reject any entry whose header
/// uncompressed size exceeds this before allocating.
pub const MAX_ENTRY_SIZE: i64 = 256 * 1024 * 1024;

/// Streaming copy buffer for extraction / zip creation (8 KiB, matching the
/// C++ `create_zip` chunk size).
const COPY_CHUNK: usize = 8192;

/// True when the entry's header uncompressed size exceeds the 256 MiB cap.
///
/// The C++ guard is `size < 0 || size > kMaxEntrySize` over a signed `int64`.
/// Header sizes here are `u64`, so the negative branch is unreachable; any
/// value above the cap (including forged multi-gigabyte sizes) is rejected.
fn exceeds_entry_cap(size: u64) -> bool {
    size > MAX_ENTRY_SIZE as u64
}

/// Clamp an archive-declared (untrusted) uncompressed size before using it as a
/// `Vec::with_capacity` hint.
///
/// The zip central directory records the uncompressed size as an
/// attacker-controlled `u64` (zip64), and `extract` streams whole entries into
/// memory. Passing that raw size straight to `with_capacity` lets a forged huge
/// value force an unbounded up-front allocation - an uncatchable
/// `handle_alloc_error` abort for a large-but-`<isize::MAX` value, or a
/// "capacity overflow" panic beyond `isize::MAX` - BEFORE a single byte is read.
/// Clamping the hint to the 256 MiB cap bounds the pre-allocation while the
/// buffer still grows to the real size as bytes stream in. The C++ `extract`
/// path streams block-by-block (`copy_data`) and never pre-allocates from the
/// header size, so this only tightens an allocation the C++ never made.
fn prealloc_hint(size: u64) -> usize {
    size.min(MAX_ENTRY_SIZE as u64) as usize
}

/// Archive format after extension routing. Mirror of the bit7z-vs-libarchive
/// split: `.7z` and `.001` -> [`Format::SevenZ`], `.rar` -> [`Format::Rar`],
/// everything else -> [`Format::Zip`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Zip,
    SevenZ,
    Rar,
}

/// Errors from the fallible archive operations (the C++ methods that
/// `throw std::runtime_error`): [`ArchiveService::extract`],
/// [`ArchiveService::extract_filtered`], [`ArchiveService::extract_prefix`],
/// [`ArchiveService::create_zip`]. The listing / read methods never surface an
/// error - they return empty results on failure, exactly like the C++.
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

/// Result of a header-only archive scan. Mirror of
/// `ArchiveService::EntryListing`.
///
/// `paths` preserves each entry's original casing and per-format separators in
/// listing order; `sizes` maps the [`normalize_path`] (lowercase,
/// forward-slash) key to the uncompressed size for case-insensitive lookup.
#[derive(Debug, Default)]
pub struct EntryListing {
    /// Entry paths in listing order (original casing / separators).
    pub paths: Vec<String>,
    /// Normalized path -> uncompressed size in bytes.
    pub sizes: HashMap<String, u64>,
}

/// Unified archive I/O facade. Stateless (a unit struct): the C++ instance
/// holds no fields either, and each call opens the archive fresh. Not intended
/// to be shared across threads while extracting; construct one per use.
#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveService;

impl ArchiveService {
    /// Construct a service handle. Cheap: there is no state.
    pub fn new() -> Self {
        ArchiveService
    }

    /// Whether an archive routes to the 7z/rar backend based on its extension.
    /// Mirror of `ArchiveService::use_bit7z`: lowercased final extension in
    /// `{.7z, .rar, .001}`.
    pub fn use_bit7z(archive_path: &str) -> bool {
        matches!(extension_lower(archive_path).as_str(), "7z" | "rar" | "001")
    }

    /// List entries with uncompressed sizes (header-only, no extraction).
    /// Mirror of `ArchiveService::list_entries_with_sizes`.
    ///
    /// Returns an empty listing on any open/read failure (never errors), as the
    /// C++ does. See the module docs for the per-format order/separator rules
    /// that reproduce the golden `archive_entries.json` byte-for-byte.
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

    /// List all entry paths (header-only). Mirror of
    /// `ArchiveService::list_entries`: the `paths` vector from
    /// [`Self::list_entries_with_sizes`] (same order / casing).
    pub fn list_entries(&self, archive_path: &str) -> Vec<String> {
        self.list_entries_with_sizes(archive_path).paths
    }

    /// Read a single entry into memory. Mirror of `ArchiveService::read_entry`.
    ///
    /// Matching is case-insensitive with normalized separators
    /// ([`normalize_path`], the Full profile). Returns an empty vector when the
    /// entry is not found, the header size exceeds the 256 MiB cap, or the
    /// archive cannot be opened (no error is surfaced).
    pub fn read_entry(&self, archive_path: &str, entry_name: &str) -> Vec<u8> {
        let target = normalize_path(entry_name);
        match format_of(archive_path) {
            Format::Zip => read_entry_zip(archive_path, &target).unwrap_or_default(),
            Format::SevenZ => read_entry_7z(archive_path, &target).unwrap_or_default(),
            Format::Rar => read_entry_rar(archive_path, &target).unwrap_or_default(),
        }
    }

    /// Read multiple entries in a single archive pass. Mirror of
    /// `ArchiveService::read_entries_batch`.
    ///
    /// `entry_names` are assumed already Full-normalized (lowercase,
    /// forward-slash), matching the C++ contract; each archive entry is
    /// normalized and looked up in the set. The returned map is keyed by the
    /// normalized path; entries not present in the archive are absent. For a
    /// solid 7z the whole block is decoded once via `for_each_entries` rather
    /// than re-decoded per entry.
    pub fn read_entries_batch(
        &self,
        archive_path: &str,
        entry_names: &HashSet<String>,
    ) -> HashMap<String, Vec<u8>> {
        // The empty check precedes the logging in the C++ (`:707-715`), so an
        // empty request emits nothing at all.
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

    /// Extract every entry to a destination directory. Mirror of
    /// `ArchiveService::extract`.
    ///
    /// Each entry is rejected (skipped, no write) when its resolved path would
    /// escape `destination_path` (the traversal guard, [`utils::is_inside`]).
    /// Errors if the archive cannot be opened.
    pub fn extract(&self, archive_path: &str, destination_path: &str) -> ArchiveResult<()> {
        // The C++ `extract` and `extract_filtered` are separate entry points with
        // separate narratives; this port implements the former via the latter, so
        // it calls the shared counted body directly rather than the public
        // `extract_filtered` (whose closing line belongs to that entry point).
        Logger::instance().log(&format!("[archive] Extracting archive: {archive_path}"));
        let count = self.extract_counted(archive_path, destination_path, |_| true)?;
        Logger::instance().log(&format!(
            "[archive] Extraction completed via {}: {count} entries",
            backend_name(format_of(archive_path))
        ));
        Ok(())
    }

    /// Extract only entries accepted by `filter`. Mirror of
    /// `ArchiveService::extract_filtered`.
    ///
    /// `filter` receives each entry's raw path (original casing / separators).
    /// The C++ runs this over a single libarchive pass regardless of format;
    /// this port routes per backend but keeps the observable contract (the
    /// filter decides which entries are written). Rejected entries are skipped
    /// without decompression where the backend allows.
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
    /// entries actually written. Shared, silent body behind [`Self::extract`],
    /// [`Self::extract_filtered`] and [`Self::extract_prefix`], each of which
    /// owns its own C++ log narrative.
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

    /// Extract entries whose normalized path starts with `prefix`. Mirror of
    /// `ArchiveService::extract_prefix`.
    ///
    /// Comparison is case-insensitive with `\`->`/` normalization on both
    /// sides. The prefix is always lowercase + `\`->`/` (matching the C++, which
    /// lowercases and slash-folds the prefix but does not strip it). The entry
    /// side is normalized PER FORMAT to mirror the two C++ code paths:
    ///
    /// - ZIP -> the C++ libarchive fallback (`ArchiveService.cpp:615-617`) uses
    ///   Light (lowercase + `\`->`/`, NO leading `./` or `/` strip), so a stored
    ///   `/textures/x` does NOT match prefix `textures` there.
    /// - 7z / rar -> the C++ bit7z path (`:584`) uses Full
    ///   ([`normalize_path`], which also strips a leading `./` and `/`).
    ///
    /// Using one uniform Full profile would make the ZIP path match strictly
    /// more entries than the C++ (any leading-`/` or `./` entry), an install
    /// replay divergence; the per-format split keeps byte parity.
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

    /// Create a zip archive from a directory tree. Mirror of
    /// `ArchiveService::create_zip`.
    ///
    /// Recursively adds every file under `folder_path` with default deflate
    /// compression; directory entries are implied, not stored. Parent
    /// directories of `output_zip_path` are created. Errors if the output file
    /// cannot be opened. Entry names use forward slashes (see PARITY-NOTES for
    /// the intentional separator divergence from the C++ `fs::relative` output).
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

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Lowercased final extension (no dot) of a path, via the last `.` in the file
/// name. Mirror of `to_lower(fs::path(p).extension().string())` minus the dot.
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
        // `.001` routes to the 7z backend (bit7z handled split volumes); no
        // `.001` fixture exists in the corpus, so this routing is UNTESTED.
        "7z" | "001" => Format::SevenZ,
        _ => Format::Zip,
    }
}

/// Normalize an entry path for [`ArchiveService::extract_prefix`] prefix
/// matching, per format. ZIP mirrors the C++ libarchive fallback
/// (`ArchiveService.cpp:615-617`): Light = lowercase + `\`->`/`, NO leading
/// strip. 7z / rar mirror the C++ bit7z path (`:584`): Full [`normalize_path`],
/// which additionally strips a leading `./` and `/`. See the module-level
/// normalization note for why the two paths must differ.
fn prefix_entry_norm(format: Format, entry_path: &str) -> String {
    match format {
        Format::Zip => to_lower(entry_path).replace('\\', "/"),
        Format::SevenZ | Format::Rar => normalize_path(entry_path),
    }
}

/// Build an [`EntryListing`] from the per-format ordered raw list: `paths`
/// keeps the original strings in order; `sizes` maps `normalize_path(path)` to
/// size (last-write-wins on a normalization collision, mirroring the C++ map
/// insert order which equals the listing order for both backends).
fn build_listing(raw: Vec<(String, u64)>) -> EntryListing {
    let mut listing = EntryListing::default();
    for (path, size) in raw {
        listing.sizes.insert(normalize_path(&path), size);
        listing.paths.push(path);
    }
    listing
}

/// Stable case-insensitive sort of (path, size) pairs by the backslash path -
/// the 7z/rar order-parity step. Stable so entries equal under lowercasing keep
/// their raw header order.
fn ci_sort_by_path(entries: &mut [(String, u64)]) {
    entries.sort_by_key(|entry| to_lower(&entry.0));
}

/// Join an entry's (possibly hostile) path onto the destination and confirm the
/// result stays inside it, mirroring the C++ traversal guard
/// (`weakly_canonical(dest/entry).lexically_relative(dest)` rejected when empty
/// or starting with `..`). Returns the safe output path, or `None` to skip.
fn safe_output_path(destination: &Path, entry_path: &str) -> Option<PathBuf> {
    let full_output = destination.join(entry_path);
    if utils::is_inside(destination, &full_output) {
        Some(full_output)
    } else {
        // The C++ emits this from both of its extraction loops
        // (`ArchiveService.cpp:307` and `:531`); routing every backend through
        // this one guard emits it from all three here.
        Logger::instance().log_warning(&format!(
            "[archive] Skipping path-traversal entry: {entry_path}"
        ));
        None
    }
}

/// The extension WITH its leading dot, lowercased, as the C++ log lines carry it
/// (`ArchiveService.cpp:402` keeps `fs::path::extension()`'s dot, so the line
/// reads "for .7z file"). An extensionless path yields "", matching the C++
/// "for  file" exactly rather than emitting a bare ".".
fn dotted_extension(archive_path: &str) -> String {
    let ext = extension_lower(archive_path);
    if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    }
}

/// Name of the crate that backs a format, for the `[archive]` log lines. The C++
/// names `bit7z` / `libarchive` in the same positions; this port has neither, so
/// the true backend is named instead (see PARITY-NOTES "Task 17").
fn backend_name(format: Format) -> &'static str {
    match format {
        Format::Zip => "zip",
        Format::SevenZ => "sevenz_rust2",
        Format::Rar => "unrar",
    }
}

/// Count an extracted entry and emit the C++ per-100 progress line
/// (`ArchiveService.cpp:343-346`).
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

// ---------------------------------------------------------------------------
// ZIP backend
// ---------------------------------------------------------------------------

/// List a zip in native central-directory order, skipping directory entries,
/// keeping the stored forward-slash names.
///
/// Directory entries ARE skipped, which empirically matches the C++ golden
/// (review finding, Task 11 - DISPROVEN): although the C++ ZIP lister is
/// libarchive and `ArchiveService.cpp:459-474` pushes every
/// `archive_entry_pathname` with no explicit `AE_IFDIR` skip, the
/// C++-generated `archive_entries.json` for the four corpus zips that DO store
/// explicit directory entries (an 11-step ZIP fixture, a SelectAtMostOne ZIP fixture,
/// a SelectExactlyOne ZIP fixture, a SelectExactlyOne ZIP fixture) contains ZERO
/// directory entries (e.g. cbbe: 466 golden entries vs 1156 stored). libarchive
/// does not surface these zip directory markers, so skipping `is_dir()` is the
/// FAITHFUL reproduction, not a coincidence; including them breaks 16/16 parity.
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
        // uncompressed size, so a forged value would otherwise abort/panic on
        // the pre-allocation. `extract` intentionally has NO 256 MiB rejection
        // (parity with the uncapped C++ streaming extract); the buffer still
        // grows to the real size via `read_to_end`.
        let mut buf = Vec::with_capacity(prealloc_hint(entry.size()));
        entry.read_to_end(&mut buf)?;
        write_extracted_file(&output, &buf)?;
        note_extracted(&mut count);
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// 7z backend
// ---------------------------------------------------------------------------

/// The backslash entry path bit7z reports for a 7z entry: sevenz_rust2 stores
/// forward slashes, so convert.
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
        // Stop once every requested entry has been collected, mirroring the C++
        // `remaining` early-out (`ArchiveService.cpp:848`). Unmatched or over-cap
        // entries (and every entry before the last match) keep the pass going;
        // `for_each_7z_entry` drains each one so the solid block stays aligned.
        Ok(results.len() < entry_names.len())
    })
    .ok()?;
    Some(results)
}

/// Decode a 7z once, invoking `each(name, size, reader)` per file entry. The
/// closure returns `Ok(true)` to continue or `Ok(false)` to stop early. This is
/// the solid-block-friendly path: `for_each_entries` decodes each block a single
/// time and streams every entry in it.
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
            // CRITICAL solid-block alignment: fully drain THIS entry's reader
            // before the block decoder advances to the next file. sevenz_rust2
            // layers every file's reader (a `BoundedReader`, optionally wrapped in
            // a `Crc32VerifyingReader`) over ONE shared per-block decode stream and
            // gives it no Drop / auto-skip (reader.rs), so the shared stream only
            // moves forward by bytes the closure actually read. A closure that read
            // none of a skipped entry (the not-matching / filtered-out / traversal
            // cases) or only part of it would leave the shared stream mid-file, and
            // the NEXT kept entry would then decode from a misaligned offset -
            // returning wrong bytes, or (when the file has a CRC) failing
            // `Crc32VerifyingReader` and surfacing as empty/absent/Err. This
            // mirrors the crate's own `read_file`, which `read_to_end`s every entry
            // precisely to stay aligned. Draining is skipped only when stopping
            // early (`keep_going == false`), where no further entry is decoded.
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
        // bit7z reports 7z paths with backslashes; feed the filter the same.
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

// ---------------------------------------------------------------------------
// RAR backend
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// create_zip
// ---------------------------------------------------------------------------

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
    // Deterministic walk: sort each directory's children so entry order is
    // stable across runs (fs read_dir order is unspecified).
    let mut files: Vec<PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        let mut children: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => continue,
        };
        children.sort();
        for child in children {
            if child.is_dir() {
                stack.push(child);
            } else {
                files.push(child);
            }
        }
    }
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
        // The C++ probes `fs::file_size` here (libarchive needs the size up
        // front) and SKIPS the entry when that fails; the zip crate needs no
        // size, so the nearest failure is the open. NOTE the divergence beyond
        // the message: the C++ `continue`s to the next file, this port
        // propagates and abandons the archive. Pre-existing, recorded in
        // PARITY-NOTES "Task 17"; not changed here because altering control flow
        // is outside a logging task.
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
            // Same shape: the C++ warns and moves to the next file, this port
            // propagates.
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
        // ZIP mirrors the C++ libarchive fallback: Light (lowercase + slash-fold,
        // NO leading-./ or -/ strip), so a stored "./x" or "/x" keeps the leading
        // segment - exactly as the C++ zip prefix filter (ArchiveService.cpp:615).
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
        // 7z / rar mirror bit7z Full normalize_path, which strips leading ./ and /.
        assert_eq!(
            prefix_entry_norm(Format::SevenZ, "./Textures/X.dds"),
            "textures/x.dds"
        );
        assert_eq!(
            prefix_entry_norm(Format::Rar, "\\Textures\\X.dds"),
            "textures/x.dds"
        );
        // The observable divergence the fix pins: prefix "textures" matches a
        // leading-"./" entry under the 7z Full profile but NOT under zip Light.
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
        // reproduces the C++ golden byte-for-byte: libarchive does not surface
        // zip directory markers, so the C++ listing omits them too (a review
        // finding claiming C++ includes them was DISPROVEN on the corpus zips
        // that store explicit dir entries - see list_zip / PARITY-NOTES).
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
        // relative `..` (both separators), a rooted/absolute name, and a
        // sibling-prefix escape (`../out-evil/*` from dest `out`, which shares a
        // name prefix with the destination but is NOT inside it).
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
        // No escaped file landed at any location the escapes actually target.
        // These are checked at their REAL resolved paths (the previous revision
        // asserted `out.join("evil_abs.txt")`, a path no build ever writes, so it
        // was vacuously true; the drive-root escape is covered deterministically
        // by `safe_output_path_rejects_all_escape_classes` below).
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
        // everything after the drive prefix -> C:\x, i.e. the DRIVE ROOT, outside
        // the destination. This is the case the item-5 guard mutation targets.
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
