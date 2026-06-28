/*!
 * @brief handles archive listing, bounded reads, extraction, and ZIP creation.
 * @author Alex (https://github.com/lextpf)
 *
 * ZIP keeps central-directory order and slash paths. 7z, split 7z, and RAR use
 * case-insensitive path order and backslash paths. these differences affect inference.
 *
 * ### :material-shield-lock: extraction and read limits
 *
 * extraction rejects entries that escape the destination. buffered read operations reject entries
 * above MAX_ENTRY_SIZE bytes.
 */

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::logger::Logger;
use crate::utils::{self, normalize_path, to_lower};

/**
 * @brief maximum decompressed entry size buffered in memory, in bytes: 256 MiB.
 * @author Alex (https://github.com/lextpf)
 *
 * the cap applies uniformly to all six read helpers (`read_entry_zip`, `_7z`, `_rar` and
 * `read_batch_zip`, `_7z`, `_rar`), so an oversized entry comes back empty from a single read and
 * absent from a batch read, whatever the format.
 */
pub const MAX_ENTRY_SIZE: i64 = 256 * 1024 * 1024;

// copy buffer for create_zip only: 8 KiB.
const COPY_CHUNK: usize = 8192;

// true when the entry's header uncompressed size exceeds the 256 MiB cap.
fn exceeds_entry_cap(size: u64) -> bool {
    size > MAX_ENTRY_SIZE as u64
}

// clamp an archive-declared (untrusted) uncompressed size before using it as a Vec::with_capacity
// hint.
// passing that raw size to `with_capacity` lets a forged value force an unbounded up-front
// allocation before a single byte is read: an uncatchable `handle_alloc_error` abort below
// `isize::MAX`, a "capacity overflow" panic beyond it.
fn prealloc_hint(size: u64) -> usize {
    size.min(MAX_ENTRY_SIZE as u64) as usize
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Zip,
    SevenZ,
    Rar,
}

/**
 * @enum ArchiveError
 * @brief errors returned by extraction and ZIP creation.
 * @author Alex (https://github.com/lextpf)
 *
 * the listing and read methods never surface an error; they return empty results on failure.
 */
#[derive(Debug)]
pub enum ArchiveError {
    /**
     * @brief the archive could not be opened or a fatal read error occurred.
     * @author Alex (https://github.com/lextpf)
     */
    Open(String),
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

pub type ArchiveResult<T> = Result<T, ArchiveError>;

/**
 * @struct EntryListing
 * @brief preserve backend path spelling and order, with sizes keyed by normalized path.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Default)]
pub struct EntryListing {
    pub paths: Vec<String>,
    pub sizes: HashMap<String, u64>,
}

/**
 * @struct ArchiveService
 * @brief allow concurrent extraction only when destination directories differ.
 * @author Alex (https://github.com/lextpf)
 *
 */
#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveService;

impl ArchiveService {
    pub fn new() -> Self {
        ArchiveService
    }

    pub fn use_bit7z(archive_path: &str) -> bool {
        matches!(extension_lower(archive_path).as_str(), "7z" | "rar" | "001")
    }

    /**
     * @fn list_entries_with_sizes(&self, &str) -> EntryListing
     * @brief list entries with uncompressed sizes, reading headers only.
     * @author Alex (https://github.com/lextpf)
     *
     * @return an empty listing on any open or read failure; it never errors, so an unreadable
     * archive is indistinguishable from an empty one.
     */
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

    /**
     * @fn list_entries(&self, &str) -> Vec<String>
     * @brief list archive entry paths in backend order and casing.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn list_entries(&self, archive_path: &str) -> Vec<String> {
        self.list_entries_with_sizes(archive_path).paths
    }

    /**
     * @fn read_entry(&self, &str, &str) -> Vec<u8>
     * @brief collapse missing, oversized and unreadable entries to empty bytes.
     * @author Alex (https://github.com/lextpf)
     *
     * no error is surfaced, so the three cases look alike.
     * @return an empty vector when the entry is missing, its header size exceeds the 256 MiB cap,
     * or the archive cannot be opened.
     */
    pub fn read_entry(&self, archive_path: &str, entry_name: &str) -> Vec<u8> {
        let target = normalize_path(entry_name);
        match format_of(archive_path) {
            Format::Zip => read_entry_zip(archive_path, &target).unwrap_or_default(),
            Format::SevenZ => read_entry_7z(archive_path, &target).unwrap_or_default(),
            Format::Rar => read_entry_rar(archive_path, &target).unwrap_or_default(),
        }
    }

    /**
     * @fn read_entries_batch(&self, &str, &HashSet<String>) -> HashMap<String, Vec<u8>>
     * @brief require normalized names and read all matches in one archive pass.
     * @author Alex (https://github.com/lextpf)
     *
     * `entry_names` must already be fully normalized (lowercase, forward-slash); each archive entry
     * is normalized and looked up in that set.
     */
    pub fn read_entries_batch(
        &self,
        archive_path: &str,
        entry_names: &HashSet<String>,
    ) -> HashMap<String, Vec<u8>> {
        // this check runs before the log line, so an empty request emits nothing at all and never
        // opens the archive.
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

    /**
     * @fn extract(&self, &str, &str) -> ArchiveResult<()>
     * @brief skip entries whose resolved path escapes the destination root.
     * @author Alex (https://github.com/lextpf)
     *
     * an entry whose resolved path would escape `destination_path` is skipped and never written
     * (the traversal guard, [`utils::is_inside`]).
     */
    pub fn extract(&self, archive_path: &str, destination_path: &str) -> ArchiveResult<()> {
        // calls the shared counted body rather than `extract_filtered`, whose closing log line
        // belongs to that entry point alone.
        Logger::instance().log(&format!("[archive] Extracting archive: {archive_path}"));
        let count = self.extract_counted(archive_path, destination_path, |_| true)?;
        Logger::instance().log(&format!(
            "[archive] Extraction completed via {}: {count} entries",
            backend_name(format_of(archive_path))
        ));
        Ok(())
    }

    /**
     * @fn extract_filtered<F>(&self,&str,&str,F)->ArchiveResult<()> where F:FnMut(&str)->bool
     * @brief preserve backend-native separators in filter input.
     * @author Alex (https://github.com/lextpf)
     *
     * a filter that must work for every format therefore has to be separator-insensitive, for
     * example by running [`normalize_path`] on its argument before matching.
     */
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

    // route to the per-format extraction backend and return the number of entries actually written.
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

    /**
     * @fn create_zip(&self, &str, &str) -> ArchiveResult<()>
     * @brief write forward-slash entry names in deterministic path order.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn create_zip(&self, folder_path: &str, output_zip_path: &str) -> ArchiveResult<()> {
        create_zip_impl(folder_path, output_zip_path)
    }

    // collect the raw (original-casing path, size) list for a format, applying the per-format
    // directory-strip / separator / ordering rules.
    // errors only on open failure; the public listing methods map that to an empty result.
    fn list_raw(&self, archive_path: &str) -> ArchiveResult<Vec<(String, u64)>> {
        match format_of(archive_path) {
            Format::Zip => list_zip(archive_path),
            Format::SevenZ => list_7z(archive_path),
            Format::Rar => list_rar(archive_path),
        }
    }
}

// lowercased final extension of a path, without the dot, from Path::extension.
fn extension_lower(archive_path: &str) -> String {
    Path::new(archive_path)
        .extension()
        .map(|e| to_lower(&e.to_string_lossy()))
        .unwrap_or_default()
}

fn format_of(archive_path: &str) -> Format {
    match extension_lower(archive_path).as_str() {
        "rar" => Format::Rar,
        // sevenz_rust2 0.21.3 cannot read multiple volumes. a genuine `.001` first volume therefore
        // fails when its end header is stored in a later volume.
        "7z" | "001" => Format::SevenZ,
        _ => Format::Zip,
    }
}

// normalize an entry path for ArchiveService::extract_prefix matching, per format.
fn prefix_entry_norm(format: Format, entry_path: &str) -> String {
    match format {
        Format::Zip => to_lower(entry_path).replace('\\', "/"),
        Format::SevenZ | Format::Rar => normalize_path(entry_path),
    }
}

// build an EntryListing from the per-format ordered raw list.
// `paths` keeps the original strings in order; `sizes` maps `normalize_path(path)` to the
// uncompressed size in bytes.
fn build_listing(raw: Vec<(String, u64)>) -> EntryListing {
    let mut listing = EntryListing::default();
    for (path, size) in raw {
        listing.sizes.insert(normalize_path(&path), size);
        listing.paths.push(path);
    }
    listing
}

// stable case-insensitive sort of (path, size) pairs by the backslash path, the ordering step for
// 7z and rar.
// stable, so entries equal under lowercasing keep their raw header order.
fn ci_sort_by_path(entries: &mut [(String, u64)]) {
    entries.sort_by_key(|entry| to_lower(&entry.0));
}

// join an entry's (possibly hostile) path onto the destination and confirm the result stays inside
// it.
// a genuine traversal: the joined path resolves outside `destination`.
fn safe_output_path(destination: &Path, entry_path: &str) -> Option<PathBuf> {
    let full_output = destination.join(entry_path);
    if utils::is_inside(destination, &full_output) {
        Some(full_output)
    } else {
        // all three extraction backends route through this guard, so the warning has a single call
        // site.
        Logger::instance().log_warning(&format!(
            "[archive] Skipping path-traversal entry: {entry_path}"
        ));
        None
    }
}

// the extension with its leading dot, lowercased, as the archive log lines carry it: "for .7z
// file".
fn dotted_extension(archive_path: &str) -> String {
    let ext = extension_lower(archive_path);
    if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    }
}

fn backend_name(format: Format) -> &'static str {
    match format {
        Format::Zip => "zip",
        Format::SevenZ => "sevenz_rust2",
        Format::Rar => "unrar",
    }
}

fn note_extracted(count: &mut usize) {
    *count += 1;
    if *count % 100 == 0 {
        Logger::instance().log(&format!("[archive] Extracted {count} files..."));
    }
}

// write a file's bytes to output, creating parent directories first.
fn write_extracted_file(output: &Path, bytes: &[u8]) -> ArchiveResult<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(output)?;
    file.write_all(bytes)?;
    Ok(())
}

// list a zip in native central-directory order, skipping directory entries and keeping the stored
// forward-slash names.
// a zip may store explicit directory markers, and the listing must contain files only.
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
        // clamp the capacity hint: `entry.size()` is the archive-controlled uncompressed size, so a
        // forged value would otherwise abort or panic on the pre-allocation.
        let mut buf = Vec::with_capacity(prealloc_hint(entry.size()));
        entry.read_to_end(&mut buf)?;
        write_extracted_file(&output, &buf)?;
        note_extracted(&mut count);
    }
    Ok(count)
}

fn sevenz_backslash_name(name: &str) -> String {
    name.replace('/', "\\")
}

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
    // match not found -> empty (never None-on-not-found; only None on open err).
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
        // stop once every requested entry has been collected. unmatched or over-cap entries, and
        // every entry before the last match, keep the pass going; `for_each_7z_entry` drains each
        // one so the solid block stays aligned.
        Ok(results.len() < entry_names.len())
    })
    .ok()?;
    Some(results)
}

// decode a 7z once. entries in one solid block share a decode cursor.
//
// [ a bytes ][ b bytes ][ c bytes ]
//   ^ partial read
//             ^ draining aligns the next reader
//
// without the drain, b starts inside a and can yield corrupt bytes or a CRC error. a false
// callback stops traversal, so no later entry needs alignment.
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
            // drain unread bytes so the next entry starts at its boundary.
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
        // the filter contract gives 7z entries backslash separators.
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
            // surface the first write error after the decode loop unwinds.
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

fn rar_name(header: &unrar::FileHeader) -> String {
    header.filename.to_string_lossy().into_owned()
}

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
    let mut files: Vec<PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        let mut children: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => continue,
        };
        // only orders the stack pushes; the global sort below overrides it.
        children.sort();
        for child in children {
            if child.is_dir() {
                stack.push(child);
            } else {
                files.push(child);
            }
        }
    }
    // this sort is what fixes the zip entry order: entries are written in `files` order, so the
    // same tree always yields the same archive layout.
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
        // the warning says "skipping", but the `?` propagates and abandons the partially written
        // archive.
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
            // same shape as the open above: warn, then propagate and abandon the archive.
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

    #[test]
    fn entry_cap_constant_is_256_mib() {
        assert_eq!(MAX_ENTRY_SIZE, 256 * 1024 * 1024);
        assert_eq!(MAX_ENTRY_SIZE, 268_435_456);
    }

    #[test]
    fn entry_cap_guard_boundary() {
        assert!(!exceeds_entry_cap(MAX_ENTRY_SIZE as u64));
        assert!(!exceeds_entry_cap(0));
        assert!(exceeds_entry_cap(MAX_ENTRY_SIZE as u64 + 1));
        assert!(exceeds_entry_cap(u64::MAX));
    }

    #[test]
    fn prealloc_hint_clamps_to_cap() {
        assert_eq!(prealloc_hint(0), 0);
        assert_eq!(prealloc_hint(1024), 1024);
        assert_eq!(
            prealloc_hint(MAX_ENTRY_SIZE as u64),
            MAX_ENTRY_SIZE as usize
        );
        // over the cap - a forged multi-gigabyte or u64::MAX uncompressed size cannot force an
        // unbounded pre-allocation; the hint saturates at the cap (the buffer still grows to the
        // real size via read_to_end).
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
        assert_eq!(
            prefix_entry_norm(Format::SevenZ, "./Textures/X.dds"),
            "textures/x.dds"
        );
        assert_eq!(
            prefix_entry_norm(Format::Rar, "\\Textures\\X.dds"),
            "textures/x.dds"
        );
        let leading = "./textures/x.dds";
        assert!(!prefix_entry_norm(Format::Zip, leading).starts_with("textures"));
        assert!(prefix_entry_norm(Format::SevenZ, leading).starts_with("textures"));
    }

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

        assert_eq!(listing.paths, vec!["a.txt", "sub/b.bin", "c.dat"]);
        assert_eq!(listing.sizes["a.txt"], 5);
        assert_eq!(listing.sizes["sub/b.bin"], 4);
        assert_eq!(listing.sizes["c.dat"], 0);
        assert_eq!(svc.list_entries(zip_path.to_str().unwrap()), listing.paths);

        fs::remove_dir_all(&dir).ok();
    }

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

        assert_eq!(
            svc.read_entry(path, "fomod/moduleconfig.xml"),
            b"<config/>".to_vec()
        );
        assert_eq!(
            svc.read_entry(path, "FOMOD\\MODULECONFIG.XML"),
            b"<config/>".to_vec()
        );
        // missing entry -> empty (not an error).
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

        assert!(svc.read_entries_batch(path, &HashSet::new()).is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extract_rejects_path_traversal_entries() {
        let dir = temp_dir("traversal");
        let zip_path = dir.join("evil.zip");
        // benign file plus malicious names that try to escape the destination: relative `..` (both
        // separators), a rooted or absolute name, and a sibling-prefix escape (`../out-evil/*` from
        // dest `out`, which shares a name prefix with the destination but is not inside it).
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

        assert!(out.join("safe/benign.txt").exists());
        // no escaped file landed where the escapes actually target. assert at the resolved paths,
        // not at `out.join("evil_abs.txt")`: nothing ever writes there, so such an assertion passes
        // vacuously. the drive-root escape is covered deterministically by
        // `safe_output_path_rejects_all_escape_classes` below.
        assert!(!dir.join("evil_rel.txt").exists());
        assert!(!dir.join("evil_bs.txt").exists());
        assert!(!dir.join("out-evil").exists());
        assert!(!dir.join("out-evil/sibling.txt").exists());
        let found = walk(&out);
        assert_eq!(found.len(), 1, "unexpected extracted files: {found:?}");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn safe_output_path_rejects_all_escape_classes() {
        // deterministic, side-effect-free proof that the traversal guard rejects every escape
        // class, including the absolute drive-root case the extraction test cannot reliably observe
        // (a weakened guard would write to C:\evil_abs.txt, invisible to a walk of the destination
        // tree).
        let dir = temp_dir("safeout");
        let dest = dir.join("out");
        fs::create_dir_all(&dest).expect("mkdir dest");

        assert!(safe_output_path(&dest, "a/b.txt").is_some());
        assert!(safe_output_path(&dest, "deep/nested/c.dat").is_some());
        // parent-directory traversal, both separators.
        assert!(safe_output_path(&dest, "../evil_rel.txt").is_none());
        assert!(safe_output_path(&dest, "..\\evil_bs.txt").is_none());
        // rooted-but-driveless names: on windows dest.join("/x") replaces everything after the
        // drive prefix, giving C:\x at the drive root, outside the destination. a weakened guard
        // would write there unnoticed.
        assert!(safe_output_path(&dest, "/evil_abs.txt").is_none());
        assert!(safe_output_path(&dest, "\\evil_abs.txt").is_none());
        // sibling-prefix escape: dest ".../out", entry resolves to ".../out-evil/x".
        assert!(safe_output_path(&dest, "../out-evil/payload.txt").is_none());

        fs::remove_dir_all(&dir).ok();
    }

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
        assert_eq!(
            svc.read_entry(zip_path.to_str().unwrap(), "nested/deep.bin"),
            vec![1u8, 2u8]
        );

        fs::remove_dir_all(&dir).ok();
    }
}
