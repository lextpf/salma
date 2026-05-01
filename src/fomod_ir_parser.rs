//! Parse `fomod/ModuleConfig.xml` bytes into the [`FomodInstaller`] IR.
//!
//! roxmltree does the parsing, but what this module accepts, and the text it
//! reads back out, is pugixml's behavior under pugixml's default parse options.
//! That is the reference the rest of the engine is calibrated against: a
//! document pugixml loads has to load here too, and has to yield the same
//! attribute and text values, or inference and install replay disagree with the
//! installed trees they are trying to reproduce. roxmltree is stricter in some
//! places and more lenient in others, so every gap is closed deliberately, and
//! the unit tests at the bottom of this file pin each case. Divergences that
//! remain on purpose are listed in PARITY-NOTES.md.
//!
//! Four stages, then one wrapper that runs all four:
//!
//! ```text
//! bytes -> decode_xml_bytes -> pugixml_lenient_pass -> load_document -> parse -> FomodInstaller
//!          detect encoding      rewrite what              depth guard,   no <config> root
//!          strip the BOM        pugixml tolerates         then roxmltree -> default installer
//!              |                     |                         |                 |
//!              v                     v                         v                 v
//!          Err(Decode)           infallible                Err(TooDeep)       infallible
//!                                                         Err(Parse)
//! ```
//!
//! - [`decode_xml_bytes`]: detect the encoding from the BOM, the leading byte
//!   pattern, or the declaration, transcode to UTF-8, and strip the BOM
//!   scalar. Fallible.
//! - `pugixml_lenient_pass` (internal, applied by [`parse_module_config`]):
//!   rewrites the malformed constructs pugixml tolerates (bare `&`, unknown
//!   entity references, `--` inside comments, stray `]]>` in character data,
//!   misplaced or malformed `<?xml ...?>` declarations) into well-formed
//!   equivalents with the same observable semantics, so roxmltree accepts
//!   them. Infallible.
//! - [`load_document`]: roxmltree parse of the decoded text, guarded by
//!   [`MAX_ELEMENT_DEPTH`] because roxmltree recurses once per element nesting
//!   level. Fallible.
//! - [`parse`]: document plus archive prefix to [`FomodInstaller`].
//!   Infallible; a document without a `<config>` root element yields a
//!   default-constructed installer.
//! - [`parse_module_config`]: the four stages in one call, keeping the
//!   roxmltree borrow internal so callers pass bytes and receive an owned IR.
//!
//! Calling [`load_document`] on its own skips the lenient pass, with two
//! consequences: malformed input that pugixml tolerates is rejected instead of
//! rewritten, and DTD-declared entity references are no longer neutralized
//! before roxmltree expands them, because [`load_document`] enables
//! `allow_dtd`.
//!
//! Names such as `guess_buffer_encoding`, `strconv_pcdata` and `parse_question`
//! in the comments below identify the pugixml 1.15 function whose behavior the
//! adjacent code reproduces. pugixml is not a dependency of this crate; the
//! names are there so the reproduced behavior can be looked up, not so the
//! source can be opened from a checkout.

use std::borrow::Cow;
use std::fmt;

use roxmltree::{Document, Node, ParsingOptions};

use crate::fomod_dependency_evaluator::MAX_DEPENDENCY_DEPTH;
use crate::fomod_ir::{
    FomodCondition, FomodConditionOp, FomodConditionType, FomodConditionalPattern, FomodFileEntry,
    FomodGroup, FomodInstaller, FomodPlugin, FomodStep, FomodTypePattern, parse_condition_op,
    parse_group_type,
};
use crate::logger::Logger;
use crate::utils::{
    get_ordered_nodes, normalize_path, parse_plugin_type_string, resolve_file_destination,
    xml_bool_attribute_true,
};

/// Maximum element children `compile_condition_impl` reads from a single
/// `<dependencies>` node. It keeps millions of sibling nodes from becoming a
/// CPU-bound denial of service.
///
/// This constant bounds breadth only. Two other bounds guard this file and are
/// easy to confuse with it:
///
/// - [`MAX_DEPENDENCY_DEPTH`] bounds the `compile_condition_impl` recursion,
///   that is, how deeply `<dependencies>` elements may nest inside each other.
/// - [`MAX_ELEMENT_DEPTH`] bounds document-level element nesting of any kind,
///   and is checked before roxmltree ever parses.
const MAX_CONDITION_CHILDREN: i32 = 10000;

/// Maximum element nesting depth accepted by [`load_document`].
///
/// roxmltree 0.21.1 recurses once per nesting level inside `Document::parse`,
/// so a crafted deeply nested ModuleConfig.xml aborts the process with an
/// uncatchable STATUS_STACK_OVERFLOW. That is not a Rust panic, so the
/// `catch_unwind` at the C ABI boundary cannot contain it. Measured against the
/// pinned roxmltree on the default 1 MiB Windows main-thread stack: a debug
/// build parses total depth 65 but dies at 81; a release build dies around
/// 2000. Checking the bound before roxmltree runs turns that abort into
/// [`FomodXmlError::TooDeep`]. pugixml parses arbitrarily deep documents, so
/// this bound is a deliberate divergence (PARITY-NOTES.md).
///
/// Why 48. The deepest schema path this parser walks before the first
/// `<dependencies>` is 11 elements:
///
/// ```text
/// config > installSteps > installStep > optionalFileGroups > group
///        > plugins > plugin > typeDescriptor > dependencyType
///        > patterns > pattern
/// ```
///
/// The outermost `<dependencies>` therefore sits at element depth 12 and
/// compiles at condition depth 0. A fully nested condition chain adds the
/// 32-level [`MAX_DEPENDENCY_DEPTH`] ceiling, reaching element depth 44, and
/// its leaf `<flagDependency>` sits at element depth 45. So 48 accepts every
/// FOMOD shape this parser can compile, with a margin of exactly 3 levels.
/// Raising [`MAX_DEPENDENCY_DEPTH`] without raising this constant makes the
/// deepest legal condition trees fail to load.
pub const MAX_ELEMENT_DEPTH: usize = 48;

/// Errors from the document-load half of the pipeline. [`parse`] itself never
/// fails, so nothing past loading can produce one of these.
#[derive(Debug)]
pub enum FomodXmlError {
    /// The byte buffer could not be transcoded to UTF-8 text under the
    /// detected encoding.
    Decode(String),
    /// roxmltree rejected the decoded document as not well-formed.
    Parse(roxmltree::Error),
    /// The document nests elements deeper than [`MAX_ELEMENT_DEPTH`], which is
    /// the carried value. roxmltree would abort the process with an uncatchable
    /// stack overflow, so [`load_document`] rejects the document up front.
    TooDeep(usize),
}

impl fmt::Display for FomodXmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FomodXmlError::Decode(msg) => write!(f, "XML transcode failed: {msg}"),
            FomodXmlError::Parse(err) => write!(f, "XML parse failed: {err}"),
            FomodXmlError::TooDeep(limit) => {
                write!(f, "XML parse failed: element nesting deeper than {limit}")
            }
        }
    }
}

impl std::error::Error for FomodXmlError {}

// ---------------------------------------------------------------------------
// Document loading: encoding detection, then pugixml's default-options
// accept set
// ---------------------------------------------------------------------------

/// The encodings [`guess_buffer_encoding`] can select for byte input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XmlEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
    Latin1,
}

/// The pugixml `ct_space` class: `\r`, `\n`, space, tab.
fn is_ct_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\r' | b' ')
}

/// The pugixml `ct_symbol` class: any byte > 127, `a-z`, `A-Z`, `0-9`, `_`,
/// `:`, `-`, `.`.
fn is_ct_symbol(b: u8) -> bool {
    b > 127 || b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'-' | b'.')
}

/// Extract the `encoding` attribute value from an XML declaration, exactly as
/// pugixml `parse_declaration_encoding` does. Only the latin1 check in
/// [`guess_buffer_encoding`] consults it.
fn parse_declaration_encoding(data: &[u8]) -> Option<&[u8]> {
    let size = data.len();
    // Check if we have a non-empty XML declaration.
    if size < 6
        || !(data[0] == b'<'
            && data[1] == b'?'
            && data[2] == b'x'
            && data[3] == b'm'
            && data[4] == b'l'
            && is_ct_space(data[5]))
    {
        return None;
    }

    // Scan the declaration until the encoding field.
    let mut i = 6usize;
    while i + 1 < size {
        // The declaration cannot contain '?' in quoted values.
        if data[i] == b'?' {
            return None;
        }
        if data[i] == b'e' && data[i + 1] == b'n' {
            let mut offset = i;
            for &ch in b"encoding" {
                if offset >= size || data[offset] != ch {
                    return None;
                }
                offset += 1;
            }
            while offset < size && is_ct_space(data[offset]) {
                offset += 1;
            }
            if offset >= size || data[offset] != b'=' {
                return None;
            }
            offset += 1;
            while offset < size && is_ct_space(data[offset]) {
                offset += 1;
            }
            // The only two valid delimiters are ' and ".
            let delimiter = if offset < size && data[offset] == b'"' {
                b'"'
            } else {
                b'\''
            };
            if offset >= size || data[offset] != delimiter {
                return None;
            }
            offset += 1;
            let start = offset;
            while offset < size && is_ct_symbol(data[offset]) {
                offset += 1;
            }
            let end = offset;
            if offset >= size || data[offset] != delimiter {
                return None;
            }
            return Some(&data[start..end]);
        }
        i += 1;
    }
    None
}

/// Detect the encoding of a byte buffer the way pugixml `guess_buffer_encoding`
/// does, following XML spec Appendix F.1. The probe order is part of the
/// contract: BOM checks first, then `<` byte-pattern probes for BOM-less
/// UTF-16/32, then a declaration `encoding=` probe that recognizes only
/// ISO-8859-1/latin1. Anything else is treated as UTF-8, and buffers shorter
/// than 4 bytes skip detection entirely and are UTF-8.
fn guess_buffer_encoding(data: &[u8]) -> XmlEncoding {
    // Skip encoding autodetection if the input buffer is too small.
    if data.len() < 4 {
        return XmlEncoding::Utf8;
    }
    let (d0, d1, d2, d3) = (data[0], data[1], data[2], data[3]);

    // Look for a BOM in the first few bytes.
    if d0 == 0 && d1 == 0 && d2 == 0xfe && d3 == 0xff {
        return XmlEncoding::Utf32Be;
    }
    if d0 == 0xff && d1 == 0xfe && d2 == 0 && d3 == 0 {
        return XmlEncoding::Utf32Le;
    }
    if d0 == 0xfe && d1 == 0xff {
        return XmlEncoding::Utf16Be;
    }
    if d0 == 0xff && d1 == 0xfe {
        return XmlEncoding::Utf16Le;
    }
    if d0 == 0xef && d1 == 0xbb && d2 == 0xbf {
        return XmlEncoding::Utf8;
    }

    // Look for <, <? or <?xm in various encodings.
    if d0 == 0 && d1 == 0 && d2 == 0 && d3 == 0x3c {
        return XmlEncoding::Utf32Be;
    }
    if d0 == 0x3c && d1 == 0 && d2 == 0 && d3 == 0 {
        return XmlEncoding::Utf32Le;
    }
    if d0 == 0 && d1 == 0x3c && d2 == 0 && d3 == 0x3f {
        return XmlEncoding::Utf16Be;
    }
    if d0 == 0x3c && d1 == 0 && d2 == 0x3f && d3 == 0 {
        return XmlEncoding::Utf16Le;
    }

    // Look for a UTF-16 '<' followed by a node name.
    if d0 == 0 && d1 == 0x3c {
        return XmlEncoding::Utf16Be;
    }
    if d0 == 0x3c && d1 == 0 {
        return XmlEncoding::Utf16Le;
    }

    // No known BOM detected; probe the declaration for latin1 spellings.
    if d0 == 0x3c && d1 == 0x3f && d2 == 0x78 && d3 == 0x6d {
        if let Some(enc) = parse_declaration_encoding(data) {
            // pugixml compares with `(c | ' ')` per letter, exact for digits
            // and '-'; eq_ignore_ascii_case accepts the same set here.
            if enc.eq_ignore_ascii_case(b"iso-8859-1") || enc.eq_ignore_ascii_case(b"latin1") {
                return XmlEncoding::Latin1;
            }
        }
    }

    XmlEncoding::Utf8
}

fn decode_utf16(bytes: &[u8], big_endian: bool) -> Result<String, FomodXmlError> {
    // A trailing odd byte is ignored: pugixml converts whole uint16_t units,
    // and chunks_exact drops the remainder the same way.
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if big_endian {
                u16::from_be_bytes([c[0], c[1]])
            } else {
                u16::from_le_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16(&units)
        .map_err(|e| FomodXmlError::Decode(format!("invalid UTF-16 sequence: {e}")))
}

fn decode_utf32(bytes: &[u8], big_endian: bool) -> Result<String, FomodXmlError> {
    bytes
        .chunks_exact(4)
        .map(|c| {
            let unit = if big_endian {
                u32::from_be_bytes([c[0], c[1], c[2], c[3]])
            } else {
                u32::from_le_bytes([c[0], c[1], c[2], c[3]])
            };
            char::from_u32(unit)
                .ok_or_else(|| FomodXmlError::Decode(format!("invalid UTF-32 unit {unit:#x}")))
        })
        .collect()
}

/// Transcode a raw ModuleConfig.xml byte buffer to UTF-8 text: detect the
/// encoding ([`guess_buffer_encoding`]), convert, and strip the leading BOM
/// scalar so it never reaches the parser.
///
/// Stricter than pugixml on malformed input by construction: invalid UTF-8,
/// lone UTF-16 surrogates and out-of-range UTF-32 units are hard errors here,
/// where pugixml passes mangled bytes through. A Rust `String` cannot hold
/// those byte sequences, so the strictness is not a choice. See
/// PARITY-NOTES.md.
pub fn decode_xml_bytes(bytes: &[u8]) -> Result<String, FomodXmlError> {
    let text = match guess_buffer_encoding(bytes) {
        XmlEncoding::Utf8 => std::str::from_utf8(bytes)
            .map_err(|e| FomodXmlError::Decode(format!("invalid UTF-8: {e}")))?
            .to_string(),
        XmlEncoding::Utf16Le => decode_utf16(bytes, false)?,
        XmlEncoding::Utf16Be => decode_utf16(bytes, true)?,
        XmlEncoding::Utf32Le => decode_utf32(bytes, false)?,
        XmlEncoding::Utf32Be => decode_utf32(bytes, true)?,
        XmlEncoding::Latin1 => bytes.iter().map(|&b| b as char).collect(),
    };
    // Drop the BOM scalar after conversion (pugixml parse_skip_bom).
    match text.strip_prefix('\u{feff}') {
        Some(stripped) => Ok(stripped.to_string()),
        None => Ok(text),
    }
}

/// Byte length of the reference at the start of `s`, which must start with
/// `&`, when pugixml's escape decoder (`strconv_escape`) and roxmltree's
/// `consume_reference` both accept it and decode it to the same scalar.
/// `None` means the two disagree and the ampersand has to be neutralized.
///
/// The agreed set is the five predefined named entities, plus character
/// references whose digits parse to an XML-valid char, with a lowercase `x`
/// hex prefix or none. Outside that set pugixml leaves the run literal, or
/// emits raw scalar bytes for XML-invalid code points, while roxmltree
/// hard-fails the load.
fn agreed_reference_len(s: &str) -> Option<usize> {
    let rest = &s[1..];
    for named in ["lt;", "gt;", "amp;", "apos;", "quot;"] {
        if rest.starts_with(named) {
            return Some(1 + named.len());
        }
    }
    let num = rest.strip_prefix('#')?;
    // Both sides accept only a lowercase 'x' hex prefix; "&#X41;" is left
    // literal by pugixml and rejected by roxmltree.
    let (digits_tail, radix, prefix_len) = match num.strip_prefix('x') {
        Some(hex) => (hex, 16u32, 3usize),
        None => (num, 10u32, 2usize),
    };
    let semi = digits_tail.find(';')?;
    let digits = &digits_tail[..semi];
    // pugixml cancels on the first non-digit before the ';' (leaves the run
    // literal); roxmltree stops consuming digits and then fails on the ';'.
    if digits.is_empty() || !digits.bytes().all(|b| char::from(b).is_digit(radix)) {
        return None;
    }
    // Values that overflow u32: pugixml accumulates with unsigned wraparound,
    // so `&#x100000041;` decodes to "A" there, while roxmltree rejects the
    // reference outright. No agreement, so neutralize and keep the literal
    // text (a residual divergence, see PARITY-NOTES.md).
    let code = u32::from_str_radix(digits, radix).ok()?;
    // Surrogates and anything beyond U+10FFFF: pugixml writes mangled UTF-8,
    // roxmltree substitutes U+FFFD. No agreement, so keep the literal text.
    let ch = char::from_u32(code)?;
    // roxmltree's XML Char check: control chars other than \t \n \r and
    // U+FFFE/U+FFFF hard-fail there, while pugixml emits the raw scalar.
    let is_xml_char = if (ch as u32) < 0x20 {
        matches!(ch, '\t' | '\n' | '\r')
    } else {
        !matches!(ch as u32, 0xFFFE | 0xFFFF)
    };
    is_xml_char.then_some(prefix_len + semi + 1)
}

/// Byte length of the DOCTYPE declaration at the start of `s` (which must
/// start with `<!DOCTYPE`), honoring quoted strings and the bracketed
/// internal subset; `s.len()` when unterminated (both parsers reject that).
fn doctype_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = "<!DOCTYPE".len();
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'[' => depth += 1,
                b']' => depth -= 1,
                b'>' if depth <= 0 => return i + 1,
                _ => {}
            },
        }
        i += 1;
    }
    s.len()
}

/// Rewrite decoded XML text so roxmltree accepts the malformed constructs
/// pugixml tolerates under default parse options, preserving pugixml's
/// observable semantics. pugixml loads such input, so rejecting it here would
/// fail packages the engine is expected to install (PARITY-NOTES.md). Four
/// rewrites:
///
/// - Every `&` that does not begin a reference both parsers agree on
///   ([`agreed_reference_len`]) becomes `&amp;`. pugixml's escape decoder
///   cancels on bare ampersands and on unknown or undeclared entity
///   references, leaving them literal in the value, and the escaped form
///   decodes back to exactly that literal text. This also neutralizes
///   references to entities declared in an internal DTD before roxmltree can
///   expand them, matching pugixml, which skips the DOCTYPE and keeps `&name;`
///   literal.
/// - A comment body roxmltree rejects (`--` inside, or a trailing `-`) is
///   blanked. pugixml scans only for the first `-->` and, with default
///   options, drops comments from the tree entirely, so the body is
///   unobservable; the span boundary at that first `-->` is preserved.
/// - A stray `]]>` outside CDATA, comments, DOCTYPE and PIs becomes `]]&gt;`.
///   pugixml's pcdata scanner (`strconv_pcdata`) stops only at `<`, `&` and
///   `\r`, so the run is ordinary character data there, while roxmltree
///   forbids `]]>` in character data. The escaped form decodes back to the
///   same literal text, inside attribute values too, where both parsers
///   already accept it raw and the rewrite changes no meaning.
/// - An XML declaration (`<?xml` followed by whitespace, up to the first `?>`)
///   becomes the `<!-- -->` placeholder. With `parse_declaration` and
///   `parse_pi` both off, which is the default, pugixml skips every `<?...?>`
///   span by scanning for the first `?>` and validates neither position nor
///   grammar (the `parse_question` skip branch), so a declaration
///   may be preceded by whitespace, lack the mandatory `version`, or sit
///   mid-document. roxmltree validates all of that and hard-fails. A comment
///   is equally absent from the pugixml tree and splits a pcdata run exactly
///   like the skipped span did. Valid declarations are rewritten too: they are
///   unobservable either way, and this avoids replicating roxmltree's
///   declaration grammar. Other `<?...?>` targets (`<?xml-stylesheet`,
///   `<?XML`, bare `<?xml?>`) are valid roxmltree PIs, invisible to the
///   tree-view helpers below, and are copied verbatim.
///
/// CDATA sections and the DOCTYPE, internal subset included, are copied
/// verbatim. This runs before [`load_document`], so `Document::input_text` and
/// the raw-range helpers below all see the rewritten text.
fn pugixml_lenient_pass(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut changed = false;
    let mut flush = 0usize; // start of the pending verbatim span
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                let rest = &text[i..];
                if let Some(after) = rest.strip_prefix("<!--") {
                    let Some(end) = after.find("-->") else {
                        break; // unterminated comment: both parsers reject
                    };
                    let body = &after[..end];
                    if body.contains("--") || body.ends_with('-') {
                        out.push_str(&text[flush..i]);
                        out.push_str("<!-- -->");
                        changed = true;
                        flush = i + 4 + end + 3;
                    }
                    i += 4 + end + 3;
                } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
                    let Some(end) = after.find("]]>") else {
                        break; // unterminated CDATA: both parsers reject
                    };
                    i += 9 + end + 3;
                } else if rest.starts_with("<!DOCTYPE") {
                    i += doctype_len(rest);
                } else if let Some(after) = rest.strip_prefix("<?") {
                    let Some(end) = after.find("?>") else {
                        break; // unterminated PI: both parsers reject
                    };
                    // Declaration-like span: '<?xml' plus whitespace. pugixml
                    // skips it wherever it appears and however malformed;
                    // roxmltree validates it. See the doc comment.
                    let after_bytes = after.as_bytes();
                    if after.starts_with("xml")
                        && after_bytes.len() > 3
                        && is_ct_space(after_bytes[3])
                    {
                        out.push_str(&text[flush..i]);
                        out.push_str("<!-- -->");
                        changed = true;
                        flush = i + 2 + end + 2;
                    }
                    i += 2 + end + 2;
                } else {
                    i += 1;
                }
            }
            b']' if text[i..].starts_with("]]>") => {
                // Stray CDATA terminator in character data: literal in
                // pugixml, forbidden by roxmltree. Real CDATA sections,
                // comments, DOCTYPEs, and PIs were skipped wholesale above,
                // so this ']' is genuine content.
                out.push_str(&text[flush..i]);
                out.push_str("]]&gt;");
                changed = true;
                i += 3;
                flush = i;
            }
            b'&' => match agreed_reference_len(&text[i..]) {
                Some(len) => i += len,
                None => {
                    out.push_str(&text[flush..i]);
                    out.push_str("&amp;");
                    changed = true;
                    i += 1;
                    flush = i;
                }
            },
            _ => i += 1,
        }
    }
    if !changed {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[flush..]);
    Cow::Owned(out)
}

/// True when the document's element nesting depth exceeds
/// [`MAX_ELEMENT_DEPTH`].
///
/// One iterative pass, no recursion, because it has to survive the input it is
/// there to reject. Comments, CDATA sections, the DOCTYPE with its internal
/// subset, and PIs are skipped wholesale; start tags are scanned to their
/// closing `>` honoring quoted attribute values, inside which raw `>` and `/`
/// are legal; self-closing tags add no depth. Unterminated constructs report
/// "not exceeded" so roxmltree can produce its own, more precise error. The
/// count is exact for well-formed input, and anything this scan miscounts is
/// not well-formed and fails the roxmltree parse anyway.
fn element_depth_exceeds(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if let Some(after) = rest.strip_prefix("<!--") {
            let Some(end) = after.find("-->") else {
                return false; // unterminated comment: roxmltree rejects
            };
            i += 4 + end + 3;
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let Some(end) = after.find("]]>") else {
                return false; // unterminated CDATA: roxmltree rejects
            };
            i += 9 + end + 3;
        } else if rest.starts_with("<!DOCTYPE") {
            i += doctype_len(rest);
        } else if let Some(after) = rest.strip_prefix("<?") {
            let Some(end) = after.find("?>") else {
                return false; // unterminated PI: roxmltree rejects
            };
            i += 2 + end + 2;
        } else if let Some(after) = rest.strip_prefix("</") {
            depth = depth.saturating_sub(1);
            let Some(end) = after.find('>') else {
                return false; // unterminated close tag: roxmltree rejects
            };
            i += 2 + end + 1;
        } else if rest.starts_with("<!") {
            // Stray markup declaration (roxmltree rejects it); no depth.
            let Some(end) = rest.find('>') else {
                return false;
            };
            i += end + 1;
        } else {
            // Start tag (or a malformed `<` run roxmltree will reject).
            let tag = rest.as_bytes();
            let mut j = 1usize;
            let mut quote: Option<u8> = None;
            while j < tag.len() {
                let b = tag[j];
                match quote {
                    Some(q) => {
                        if b == q {
                            quote = None;
                        }
                    }
                    None => match b {
                        b'"' | b'\'' => quote = Some(b),
                        b'>' => break,
                        _ => {}
                    },
                }
                j += 1;
            }
            if j >= tag.len() {
                return false; // unterminated tag: roxmltree rejects
            }
            if tag[j - 1] != b'/' {
                depth += 1;
                if depth > MAX_ELEMENT_DEPTH {
                    return true;
                }
            }
            i += j + 1;
        }
    }
    false
}

/// Parse decoded text into a roxmltree document.
///
/// `allow_dtd` is enabled so a harmless DOCTYPE does not hard-fail where
/// pugixml would skip it. References to DTD-declared entities never reach
/// expansion, because [`parse_module_config`] neutralizes them in the lenient
/// pass first; that ordering is what makes `allow_dtd` safe here.
///
/// Element nesting deeper than [`MAX_ELEMENT_DEPTH`] is rejected as
/// [`FomodXmlError::TooDeep`] before roxmltree runs: its per-depth recursion
/// would otherwise abort the whole process with an uncatchable stack overflow
/// on input pugixml parses without complaint.
pub fn load_document(text: &str) -> Result<Document<'_>, FomodXmlError> {
    if element_depth_exceeds(text) {
        return Err(FomodXmlError::TooDeep(MAX_ELEMENT_DEPTH));
    }
    let options = ParsingOptions {
        allow_dtd: true,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options).map_err(FomodXmlError::Parse)
}

/// Bytes to an owned [`FomodInstaller`] in one call: [`decode_xml_bytes`],
/// `pugixml_lenient_pass`, [`load_document`], [`parse`]. The roxmltree document
/// borrows the decoded text, so that borrow stays inside this function and the
/// inference and install services hand over bytes and receive an owned IR.
pub fn parse_module_config(
    bytes: &[u8],
    archive_prefix: &str,
) -> Result<FomodInstaller, FomodXmlError> {
    let text = decode_xml_bytes(bytes)?;
    let text = pugixml_lenient_pass(&text);
    let doc = load_document(&text)?;
    Ok(parse(&doc, archive_prefix))
}

// ---------------------------------------------------------------------------
// pugixml tree-view helpers
//
// Under default parse options pugixml's tree holds elements, non-whitespace
// pcdata and cdata nodes. Comments, PIs, the declaration and whitespace-only
// pcdata are absent from it. roxmltree keeps all of them, so these helpers
// reproduce the pugixml view. On name matching: pugixml compares raw qualified
// names, and roxmltree local names match them for every un-prefixed element,
// which is the only kind FOMOD schemas produce. Edge cases in PARITY-NOTES.md.
//
// The invisible fact all three text helpers turn on: roxmltree merges a run of
// adjacent pcdata and CDATA chunks into one text node whose range() covers only
// the first raw chunk. pugixml keeps the chunks as separate nodes.
//
//   source:    <flag>  abc<![CDATA[def]]>tail</flag>
//                     ^^^^^^^^^^^^^^^^^^^^^^^^^
//   roxmltree: [ one text node, decoded value "  abcdeftail" ]
//              range() covers only the first raw chunk: "  abc"
//   pugixml:   [pcdata "  abc"] [cdata "def"] [pcdata "tail"]
//              node.text() -> "  abc"   (first node in the pugi tree)
//
// Whitespace-only chunk before a CDATA section:
//
//   source:    <flag>  <![CDATA[ ]]></flag>
//   roxmltree: [ one text node "   " ]
//   pugixml:   [pcdata dropped] [cdata " "]  -> node.text() = " "
//              (a cdata node exists even when it is whitespace-only or empty)
//
// So text_node_exists_in_pugi_tree and pugi_text walk the raw input chunks
// instead of trusting the merged node: existence is decided on raw source
// chars, while the value comes from the decoded text.
// ---------------------------------------------------------------------------

/// First element child with the given name, as pugixml `node.child(name)`.
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

/// Element children with the given name, in document order, as pugixml
/// `node.children(name)`.
fn named_children<'a, 'input>(
    node: Node<'a, 'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

/// Attribute value with a fallback, as pugixml
/// `attribute(name).as_string(def)`. An attribute present but empty yields the
/// empty string, not the fallback.
fn attr_or<'a>(node: Node<'a, '_>, name: &str, default: &'a str) -> &'a str {
    node.attribute(name).unwrap_or(default)
}

/// True when the string is empty or all pugixml `ct_space` bytes.
///
/// Pass the raw input slice, never the decoded text. pugixml decides whether to
/// drop a pcdata run on the raw source chars, before escape expansion, because
/// `PUGI_IMPL_SKIPWS` stops at the `&` of a character reference. So raw `&#32;`
/// is a pcdata node whose decoded value is " ", not a dropped run, and passing
/// decoded text here would drop it.
fn is_ws_only(s: &str) -> bool {
    s.bytes().all(is_ct_space)
}

/// True when this roxmltree text node would exist in the pugixml tree. A chunk
/// adjacent to a CDATA section always yields a pugixml node, since cdata is
/// kept even when whitespace-only; otherwise pcdata whose raw source chars are
/// whitespace-only is dropped (see [`is_ws_only`]).
fn text_node_exists_in_pugi_tree(doc: &Document, node: Node) -> bool {
    let input = doc.input_text();
    let range = node.range();
    input[range.start..].starts_with("<![CDATA[")
        || input[range.end..].starts_with("<![CDATA[")
        || !is_ws_only(&input[range.start..range.end])
}

/// pugixml `node.first_child()` truthiness: true for any element, cdata or
/// non-whitespace pcdata child.
fn pugi_has_first_child(doc: &Document, node: Node) -> bool {
    node.children()
        .any(|c| c.is_element() || (c.is_text() && text_node_exists_in_pugi_tree(doc, c)))
}

/// Normalize line endings as pugixml `parse_eol` does: `\r\n` to `\n`, lone
/// `\r` to `\n`. Only the raw-chunk path below needs it; roxmltree already
/// normalizes its decoded text.
fn normalize_eol(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Decode one raw pcdata chunk the way pugixml's `parse_escapes` plus
/// `parse_eol` do: decode the five predefined entities and character
/// references, leave anything unrecognized literal, and normalize line endings
/// on the literal segments only.
///
/// Normalizing the whole chunk would be wrong: a reference-produced `\r`, such
/// as `&#13;`, survives in both pugixml and roxmltree and must not be rewritten
/// to `\n`. Only chunks roxmltree merged into a neighboring CDATA section come
/// through here. The document has already parsed by then, so undeclared
/// entities cannot reach this code, and the literal-`&` fallback is defensive.
fn decode_pcdata_chunk(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&normalize_eol(&rest[..amp]));
        let tail = &rest[amp..];
        let decoded = tail.find(';').and_then(|semi| {
            let entity = &tail[1..semi];
            let ch = match entity {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "apos" => Some('\''),
                "quot" => Some('"'),
                _ => entity.strip_prefix("#x").map_or_else(
                    || {
                        entity
                            .strip_prefix('#')
                            .and_then(|d| d.parse::<u32>().ok())
                            .and_then(char::from_u32)
                    },
                    |h| u32::from_str_radix(h, 16).ok().and_then(char::from_u32),
                ),
            };
            ch.map(|c| (c, semi))
        });
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(&normalize_eol(rest));
    out
}

/// The value of the first pcdata or cdata child in the pugixml tree, as
/// pugixml `node.text().as_string()`. Never concatenated mixed content:
/// element children are skipped over, the way pugixml's `xml_text` scans past
/// them, and whitespace-only pcdata does not exist in that tree.
///
/// roxmltree merges directly adjacent pcdata and CDATA runs into one text node,
/// so a run that involves CDATA is walked chunk by chunk the way the pugixml
/// tree stores it. The common no-CDATA case uses roxmltree's decoded text
/// directly. Returns an empty string when the element has no such child.
fn pugi_text(doc: &Document, node: Node) -> String {
    let input = doc.input_text();
    for node_child in node.children() {
        if !node_child.is_text() {
            continue;
        }
        let range = node_child.range();
        let starts_cdata = input[range.start..].starts_with("<![CDATA[");
        if !starts_cdata && !input[range.end..].starts_with("<![CDATA[") {
            // Plain pcdata run (the common case): existence is decided on the
            // raw source chars, because pugixml drops ws-only pcdata before
            // escape expansion, so raw `&#32;` is a node. The value is
            // roxmltree's decoded text, exactly pugixml's pcdata value.
            if is_ws_only(&input[range.start..range.end]) {
                continue; // not a node in the pugixml tree
            }
            return node_child.text().unwrap_or("").to_string();
        }
        // Merged pcdata/CDATA run: walk the raw chunks like the pugixml tree.
        let mut pos = range.start;
        loop {
            if let Some(content) = input[pos..].strip_prefix("<![CDATA[") {
                // A cdata node always exists, even empty or whitespace-only.
                let content_end = content.find("]]>").unwrap_or(content.len());
                return normalize_eol(&content[..content_end]);
            }
            let chunk_end = input[pos..].find('<').map_or(input.len(), |e| pos + e);
            let raw_chunk = &input[pos..chunk_end];
            // Existence from the raw chunk, before escape expansion; value
            // from the decoded chunk. That is pugixml's drop-then-decode
            // order, and swapping it loses whitespace character references.
            if !is_ws_only(raw_chunk) {
                return decode_pcdata_chunk(raw_chunk);
            }
            // Whitespace-only pcdata is dropped; continue only if the run
            // carries on with a CDATA section.
            if input[chunk_end..].starts_with("<![CDATA[") {
                pos = chunk_end;
            } else {
                break;
            }
        }
    }
    String::new()
}

/// Read an attribute as an integer the way pugixml `as_int(0)` does. A missing
/// attribute is 0. Otherwise: skip leading `ct_space`, take an optional sign,
/// accept a `0x` or `0X` hex prefix, consume digits until the first non-digit
/// (`"abc"` gives 0, `"12abc"` gives 12), and saturate to `i32::MIN` or
/// `i32::MAX` on overflow. It never fails, so a garbage `priority` attribute
/// silently becomes 0 rather than rejecting the installer.
fn pugi_as_int(attr: Option<&str>) -> i32 {
    let Some(value) = attr else {
        return 0;
    };
    let b = value.as_bytes();
    let mut i = 0usize;
    while i < b.len() && is_ct_space(b[i]) {
        i += 1;
    }
    let negative = i < b.len() && b[i] == b'-';
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }

    let mut result: u32 = 0;
    let overflow;
    if i + 1 < b.len() && b[i] == b'0' && (b[i + 1] | 0x20) == b'x' {
        i += 2;
        // Overflow detection relies on the digit count; skip leading zeros.
        while i < b.len() && b[i] == b'0' {
            i += 1;
        }
        let start = i;
        while i < b.len() {
            let lower = b[i] | 0x20;
            let digit = if b[i].is_ascii_digit() {
                u32::from(b[i] - b'0')
            } else if (b'a'..=b'f').contains(&lower) {
                u32::from(lower - b'a' + 10)
            } else {
                break;
            };
            result = result.wrapping_mul(16).wrapping_add(digit);
            i += 1;
        }
        overflow = i - start > 8; // sizeof(u32) * 2 hex digits
    } else {
        while i < b.len() && b[i] == b'0' {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            result = result.wrapping_mul(10).wrapping_add(u32::from(b[i] - b'0'));
            i += 1;
        }
        let digits = i - start;
        // 32-bit constants: max_digits10 = 10, max_lead = '4', high_bit = 31.
        overflow = digits >= 10
            && !(digits == 10 && (b[start] < b'4' || (b[start] == b'4' && result >> 31 != 0)));
    }

    if negative {
        if overflow || result > 0x8000_0000 {
            i32::MIN
        } else {
            0u32.wrapping_sub(result) as i32
        }
    } else if overflow || result > 0x7fff_ffff {
        i32::MAX
    } else {
        result as i32
    }
}

/// Compile a `<dependencies>` element into a [`FomodCondition`] tree.
///
/// Infallible: every malformed input degrades to a well-formed condition, so
/// the caller never handles an error. The degradations are the part worth
/// knowing.
///
/// **Attribute defaults.** `operator` on the composite defaults to `"And"`,
/// `state` on `<fileDependency>` and `type` on `<pluginDependency>` default to
/// `"Active"`. Every other leaf attribute defaults to the empty string. An
/// unrecognized `operator` value parses to `And`, not to an error.
///
/// **Breadth.** Element children past [`MAX_CONDITION_CHILDREN`] are dropped
/// silently and the walk stops there. The counter increments before the
/// element name is dispatched, so an unknown element consumes a cap slot even
/// though it contributes no child.
///
/// **Depth.** A `<dependencies>` nested deeper than [`MAX_DEPENDENCY_DEPTH`]
/// does not fail: the subtree is replaced by an empty `Or` composite, which
/// evaluates to always-false, and its real children are discarded.
///
/// **Logging.** Breadth truncation, depth truncation and every skipped unknown
/// element write a warning through [`Logger`], which appends to
/// `logs/salma.log` and fires the host log callback. That is real file I/O on
/// the parse path, and on adversarial input the unknown-element warning fires
/// once per skipped child, up to [`MAX_CONDITION_CHILDREN`] lines for a single
/// condition node.
fn compile_condition(deps_node: Node) -> FomodCondition {
    compile_condition_impl(deps_node, 0)
}

fn compile_condition_impl(deps_node: Node, depth: i32) -> FomodCondition {
    let mut cond = FomodCondition {
        r#type: FomodConditionType::Composite,
        ..FomodCondition::default()
    };

    if depth > MAX_DEPENDENCY_DEPTH {
        // Reject the subtree with an always-false empty Or.
        Logger::instance().log_warning(
            "[fomod] Condition nesting exceeds maximum depth, treating as always-false",
        );
        cond.op = FomodConditionOp::Or;
        return cond;
    }

    cond.op = parse_condition_op(attr_or(deps_node, "operator", "And"));

    let mut child_count = 0i32;
    for node_child in deps_node.children() {
        if !node_child.is_element() {
            continue;
        }
        child_count += 1;
        if child_count > MAX_CONDITION_CHILDREN {
            Logger::instance().log_warning(&format!(
                "[fomod] Condition node has more than {MAX_CONDITION_CHILDREN} children, truncating"
            ));
            break;
        }

        let mut leaf = FomodCondition::default();
        let child_name = node_child.tag_name().name();
        match child_name {
            "flagDependency" => {
                leaf.r#type = FomodConditionType::Flag;
                leaf.flag_name = attr_or(node_child, "flag", "").to_string();
                leaf.flag_value = attr_or(node_child, "value", "").to_string();
                cond.children.push(leaf);
            }
            "fileDependency" => {
                leaf.r#type = FomodConditionType::File;
                leaf.file_path = attr_or(node_child, "file", "").to_string();
                leaf.file_state = attr_or(node_child, "state", "Active").to_string();
                cond.children.push(leaf);
            }
            "gameDependency" => {
                leaf.r#type = FomodConditionType::Game;
                leaf.version = attr_or(node_child, "version", "").to_string();
                cond.children.push(leaf);
            }
            "pluginDependency" => {
                leaf.r#type = FomodConditionType::Plugin;
                leaf.plugin_name = attr_or(node_child, "name", "").to_string();
                leaf.plugin_type = attr_or(node_child, "type", "Active").to_string();
                cond.children.push(leaf);
            }
            "fomodDependency" => {
                leaf.r#type = FomodConditionType::Fomod;
                leaf.fomod_name = attr_or(node_child, "name", "").to_string();
                cond.children.push(leaf);
            }
            "fommDependency" => {
                leaf.r#type = FomodConditionType::Fomm;
                leaf.version = attr_or(node_child, "version", "").to_string();
                cond.children.push(leaf);
            }
            "foseDependency" => {
                leaf.r#type = FomodConditionType::Fose;
                leaf.version = attr_or(node_child, "version", "").to_string();
                cond.children.push(leaf);
            }
            "dependencies" => {
                cond.children
                    .push(compile_condition_impl(node_child, depth + 1));
            }
            _ => {
                // `child_name` is roxmltree's local name, the same name every
                // match arm above compares against, so the dispatch and this
                // message agree. pugixml would log the qualified name, prefix
                // included; only a prefixed unknown element reads differently
                // (see PARITY-NOTES.md).
                Logger::instance().log_warning(&format!(
                    "[fomod] Unknown condition element \"{child_name}\" skipped"
                ));
            }
        }
    }

    cond
}

/// Build one [`FomodFileEntry`] from a `<file>` or `<folder>` element. See
/// [`parse`] for how `archive_prefix` and the destination fallbacks combine.
fn parse_file_entry(node: Node, archive_prefix: &str) -> FomodFileEntry {
    let mut entry = FomodFileEntry::default();
    let source_attr = attr_or(node, "source", "");
    // A present destination attribute is used verbatim, even when empty; only
    // an absent attribute falls back to the source value.
    let dest_raw = node.attribute("destination").unwrap_or(source_attr);

    let is_folder = node.tag_name().name() == "folder";
    entry.is_folder = is_folder;

    // Build the archive-relative source path.
    let full_source = if archive_prefix.is_empty() {
        source_attr.to_string()
    } else {
        format!("{archive_prefix}/{source_attr}")
    };
    entry.source = normalize_path(&full_source);

    // Resolve and normalize the destination.
    entry.destination = if is_folder {
        normalize_path(dest_raw)
    } else {
        normalize_path(&resolve_file_destination(source_attr, dest_raw, true))
    };

    entry.priority = pugi_as_int(node.attribute("priority"));
    entry.always_install = xml_bool_attribute_true(node.attribute("alwaysInstall"));
    entry.install_if_usable = xml_bool_attribute_true(node.attribute("installIfUsable"));

    entry
}

/// Iterate the `<file>` and `<folder>` child elements that carry a non-empty
/// `source` attribute. Everything else, including text nodes and elements with
/// an empty `source`, is skipped without a warning.
fn file_nodes<'a, 'input>(parent: Node<'a, 'input>) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    parent.children().filter(|node| {
        if !node.is_element() {
            return false;
        }
        let name = node.tag_name().name();
        if name != "file" && name != "folder" {
            return false;
        }
        !attr_or(*node, "source", "").is_empty()
    })
}

/// Collect the named element children of an optional parent and order them by
/// the parent's `order` attribute through
/// [`crate::utils::get_ordered_nodes`]. A `None` parent yields an empty list,
/// so a missing `<installSteps>` or `<plugins>` is not an error.
fn ordered_children<'a, 'input>(
    parent: Option<Node<'a, 'input>>,
    child_name: &str,
) -> Vec<Node<'a, 'input>> {
    let Some(parent) = parent else {
        return Vec::new();
    };
    let nodes: Vec<Node<'a, 'input>> = parent
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == child_name)
        .collect();
    get_ordered_nodes(parent.attribute("order"), nodes, |n| {
        n.attribute("name").unwrap_or("")
    })
}

/// Walk the document and build a fully populated [`FomodInstaller`].
/// Infallible: a document without a `<config>` root element returns a default
/// empty installer, and malformed or unknown elements are skipped.
///
/// `archive_prefix` is the archive-relative directory that holds the `fomod`
/// folder, and it must not end with a separator, because this function inserts
/// the `/` itself. An empty prefix prepends nothing at all, not even a
/// separator, so `""` yields source paths exactly as the XML spells them. The
/// caller in `fomod_inference_service` derives the prefix as a substring with
/// no trailing separator.
///
/// Every produced [`FomodFileEntry`] has `source` and `destination` normalized
/// by [`crate::utils::normalize_path`]: lowercase, forward slashes, no leading
/// or trailing slash. A `<file>` destination first goes through
/// [`crate::utils::resolve_file_destination`], the least obvious behavior here:
/// an empty destination becomes the source filename, and a destination ending
/// in `/` or `\` is treated as a directory and gets the source filename
/// appended. `<folder>` destinations skip that step, so an empty folder
/// destination stays empty and the folder lands at the mod root.
pub fn parse(doc: &Document, archive_prefix: &str) -> FomodInstaller {
    let mut installer = FomodInstaller::default();
    // First element child of the document named "config", as pugixml
    // doc.child("config"); comments before the root are not in that tree.
    let Some(config) = doc
        .root()
        .children()
        .find(|c| c.is_element() && c.tag_name().name() == "config")
    else {
        return installer;
    };

    // Module dependencies.
    if let Some(mod_deps) = child(config, "moduleDependencies") {
        if let Some(deps_child) = child(mod_deps, "dependencies") {
            installer.module_dependencies = Some(compile_condition(deps_child));
        } else if pugi_has_first_child(doc, mod_deps) {
            installer.module_dependencies = Some(compile_condition(mod_deps));
        }
    }

    // Required install files.
    if let Some(req_parent) = child(config, "requiredInstallFiles") {
        for node in file_nodes(req_parent) {
            installer
                .required_files
                .push(parse_file_entry(node, archive_prefix));
        }
    }

    // Install steps (ordered).
    let steps_parent = child(config, "installSteps");
    let ordered_steps = ordered_children(steps_parent, "installStep");
    for (step_ordinal, step_node) in ordered_steps.into_iter().enumerate() {
        let mut step = FomodStep {
            name: attr_or(step_node, "name", "").to_string(),
            ordinal: step_ordinal as i32,
            ..FomodStep::default()
        };

        // Step visibility.
        if let Some(visible_node) = child(step_node, "visible") {
            if let Some(deps) = child(visible_node, "dependencies") {
                step.visible = Some(compile_condition(deps));
            } else if pugi_has_first_child(doc, visible_node) {
                step.visible = Some(compile_condition(visible_node));
            }
        }

        // Groups (ordered).
        let fg_parent = child(step_node, "optionalFileGroups");
        for group_node in ordered_children(fg_parent, "group") {
            let mut group = FomodGroup {
                name: attr_or(group_node, "name", "").to_string(),
                r#type: parse_group_type(attr_or(group_node, "type", "SelectAny")),
                ..FomodGroup::default()
            };

            // Plugins (ordered).
            let plugins_parent = child(group_node, "plugins");
            for pnode in ordered_children(plugins_parent, "plugin") {
                let mut plugin = FomodPlugin {
                    name: attr_or(pnode, "name", "").to_string(),
                    ..FomodPlugin::default()
                };

                // Type descriptor.
                if let Some(type_desc) = child(pnode, "typeDescriptor") {
                    if let Some(type_node) = child(type_desc, "type") {
                        plugin.r#type =
                            parse_plugin_type_string(attr_or(type_node, "name", "Optional"));
                    } else if let Some(dep_type) = child(type_desc, "dependencyType") {
                        if let Some(default_type) = child(dep_type, "defaultType") {
                            plugin.r#type =
                                parse_plugin_type_string(attr_or(default_type, "name", "Optional"));
                        }
                        if let Some(patterns) = child(dep_type, "patterns") {
                            for pat in named_children(patterns, "pattern") {
                                let mut tp = FomodTypePattern::default();
                                if let Some(pat_deps) = child(pat, "dependencies") {
                                    tp.condition = compile_condition(pat_deps);
                                }
                                // else: default condition (empty And) = always true
                                if let Some(ptype) = child(pat, "type") {
                                    tp.result_type = parse_plugin_type_string(attr_or(
                                        ptype, "name", "Optional",
                                    ));
                                }
                                plugin.type_patterns.push(tp);
                            }
                        }
                    }
                }

                // Files.
                if let Some(files_node) = child(pnode, "files") {
                    for fnode in file_nodes(files_node) {
                        plugin.files.push(parse_file_entry(fnode, archive_prefix));
                    }
                }

                // Condition flags.
                if let Some(cf_node) = child(pnode, "conditionFlags") {
                    for flag_node in named_children(cf_node, "flag") {
                        let flag_name = attr_or(flag_node, "name", "").to_string();
                        let flag_value = pugi_text(doc, flag_node);
                        if !flag_name.is_empty() {
                            plugin.condition_flags.push((flag_name, flag_value));
                        }
                    }
                }

                // Plugin dependencies.
                if let Some(plugin_deps) = child(pnode, "dependencies") {
                    plugin.dependencies = Some(compile_condition(plugin_deps));
                }

                group.plugins.push(plugin);
            }
            step.groups.push(group);
        }
        installer.steps.push(step);
    }

    // Conditional file installs.
    if let Some(cfi) = child(config, "conditionalFileInstalls") {
        if let Some(patterns) = child(cfi, "patterns") {
            for pat in named_children(patterns, "pattern") {
                let mut cp = FomodConditionalPattern::default();
                if let Some(deps) = child(pat, "dependencies") {
                    cp.condition = compile_condition(deps);
                }
                // else: default condition (empty And) = always true
                if let Some(files) = child(pat, "files") {
                    for fnode in file_nodes(files) {
                        cp.files.push(parse_file_entry(fnode, archive_prefix));
                    }
                }
                installer.conditional_patterns.push(cp);
            }
        }
    }

    installer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PluginType;

    fn parse_str(xml: &str) -> FomodInstaller {
        parse_str_with_prefix(xml, "")
    }

    fn parse_str_with_prefix(xml: &str, prefix: &str) -> FomodInstaller {
        parse_module_config(xml.as_bytes(), prefix).expect("well-formed test XML must load")
    }

    // --- config root lookup ---

    #[test]
    fn missing_config_root_returns_empty_default_installer() {
        let installer = parse_str("<notconfig><installSteps/></notconfig>");
        assert_eq!(installer, FomodInstaller::default());
    }

    #[test]
    fn config_root_is_found_past_leading_comment() {
        let installer = parse_str(
            "<!-- header --><config><installSteps><installStep name=\"A\"><optionalFileGroups/></installStep></installSteps></config>",
        );
        assert_eq!(installer.steps.len(), 1);
        assert_eq!(installer.steps[0].name, "A");
    }

    // --- moduleDependencies dispatch ---

    #[test]
    fn module_dependencies_prefers_dependencies_child() {
        let installer = parse_str(
            "<config><moduleDependencies operator=\"Or\"><dependencies operator=\"Or\">\
             <flagDependency flag=\"f\" value=\"v\"/></dependencies>\
             </moduleDependencies></config>",
        );
        let deps = installer.module_dependencies.expect("present");
        assert_eq!(deps.op, FomodConditionOp::Or);
        assert_eq!(deps.children.len(), 1);
        assert_eq!(deps.children[0].r#type, FomodConditionType::Flag);
    }

    #[test]
    fn module_dependencies_compiles_from_itself_when_no_dependencies_child() {
        let installer = parse_str(
            "<config><moduleDependencies operator=\"Or\">\
             <fileDependency file=\"a.esp\"/></moduleDependencies></config>",
        );
        let deps = installer.module_dependencies.expect("present");
        assert_eq!(deps.op, FomodConditionOp::Or);
        assert_eq!(deps.children.len(), 1);
        assert_eq!(deps.children[0].r#type, FomodConditionType::File);
    }

    #[test]
    fn empty_module_dependencies_is_absent() {
        let installer = parse_str("<config><moduleDependencies></moduleDependencies></config>");
        assert!(installer.module_dependencies.is_none());
    }

    #[test]
    fn whitespace_only_module_dependencies_is_absent() {
        // Whitespace-only pcdata is not in the pugixml tree (parse_ws_pcdata
        // off), so first_child() is null and the branch is not taken.
        let installer =
            parse_str("<config><moduleDependencies>\n\t  \n</moduleDependencies></config>");
        assert!(installer.module_dependencies.is_none());
    }

    #[test]
    fn text_content_makes_module_dependencies_present_but_childless() {
        // Non-whitespace pcdata counts as a pugixml child, so first_child()
        // is truthy and moduleDependencies compiles from itself; the pcdata
        // is not an element, so the composite has no children.
        let installer = parse_str("<config><moduleDependencies>x</moduleDependencies></config>");
        let deps = installer.module_dependencies.expect("present");
        assert_eq!(deps.r#type, FomodConditionType::Composite);
        assert_eq!(deps.op, FomodConditionOp::And);
        assert!(deps.children.is_empty());
    }

    // --- compile_condition ---

    fn compile_from(xml: &str) -> FomodCondition {
        let installer = parse_str(xml);
        installer.module_dependencies.expect("condition present")
    }

    #[test]
    fn operator_attribute_defaults_and_misses_map_to_and() {
        let cond = compile_from(
            "<config><moduleDependencies><dependencies>\
             <flagDependency flag=\"f\" value=\"v\"/></dependencies></moduleDependencies></config>",
        );
        assert_eq!(cond.op, FomodConditionOp::And);
        let cond = compile_from(
            "<config><moduleDependencies><dependencies operator=\"or\">\
             <flagDependency flag=\"f\" value=\"v\"/></dependencies></moduleDependencies></config>",
        );
        assert_eq!(cond.op, FomodConditionOp::And, "parse_enum miss -> And");
        let cond = compile_from(
            "<config><moduleDependencies><dependencies operator=\"Or\">\
             <flagDependency flag=\"f\" value=\"v\"/></dependencies></moduleDependencies></config>",
        );
        assert_eq!(cond.op, FomodConditionOp::Or);
    }

    #[test]
    fn all_leaf_kinds_defaults_and_unknown_elements() {
        let cond = compile_from(
            "<config><moduleDependencies><dependencies>\
             <flagDependency flag=\"f\" value=\"v\"/>\
             <fileDependency file=\"p.esp\"/>\
             <fileDependency file=\"q.esp\" state=\"Missing\"/>\
             <gameDependency version=\"1.5.97\"/>\
             <pluginDependency name=\"skse\"/>\
             <pluginDependency name=\"other.esp\" type=\"Inactive\"/>\
             <fomodDependency name=\"SomeFomod\"/>\
             <fommDependency version=\"0.13\"/>\
             <foseDependency version=\"2.0\"/>\
             <notADependency foo=\"bar\"/>\
             <dependencies operator=\"Or\"><flagDependency flag=\"g\" value=\"w\"/></dependencies>\
             </dependencies></moduleDependencies></config>",
        );
        assert_eq!(cond.children.len(), 10, "unknown element skipped");

        assert_eq!(cond.children[0].r#type, FomodConditionType::Flag);
        assert_eq!(cond.children[0].flag_name, "f");
        assert_eq!(cond.children[0].flag_value, "v");

        assert_eq!(cond.children[1].r#type, FomodConditionType::File);
        assert_eq!(cond.children[1].file_path, "p.esp");
        assert_eq!(
            cond.children[1].file_state, "Active",
            "state defaults to Active"
        );
        assert_eq!(cond.children[2].file_state, "Missing");

        assert_eq!(cond.children[3].r#type, FomodConditionType::Game);
        assert_eq!(cond.children[3].version, "1.5.97");

        assert_eq!(cond.children[4].r#type, FomodConditionType::Plugin);
        assert_eq!(cond.children[4].plugin_name, "skse");
        assert_eq!(
            cond.children[4].plugin_type, "Active",
            "type defaults to Active"
        );
        assert_eq!(cond.children[5].plugin_type, "Inactive");

        assert_eq!(cond.children[6].r#type, FomodConditionType::Fomod);
        assert_eq!(cond.children[6].fomod_name, "SomeFomod");

        assert_eq!(cond.children[7].r#type, FomodConditionType::Fomm);
        assert_eq!(cond.children[7].version, "0.13");

        assert_eq!(cond.children[8].r#type, FomodConditionType::Fose);
        assert_eq!(cond.children[8].version, "2.0");

        assert_eq!(cond.children[9].r#type, FomodConditionType::Composite);
        assert_eq!(cond.children[9].op, FomodConditionOp::Or);
        assert_eq!(cond.children[9].children.len(), 1);
    }

    #[test]
    fn text_between_condition_children_is_not_counted() {
        let cond = compile_from(
            "<config><moduleDependencies><dependencies>text\
             <flagDependency flag=\"f\" value=\"v\"/>more text\
             </dependencies></moduleDependencies></config>",
        );
        assert_eq!(
            cond.children.len(),
            1,
            "only element children are processed"
        );
    }

    #[test]
    fn depth_beyond_max_becomes_always_false_or() {
        // Root <dependencies> compiles at depth 0; a node nested k levels deep
        // compiles at depth k; depth 33 > MAX_DEPENDENCY_DEPTH(32) bails out
        // with an empty Or (always false), dropping its real children.
        let nest_levels = (MAX_DEPENDENCY_DEPTH + 1) as usize; // node at depth 33 exists
        let mut xml = String::from("<config><moduleDependencies><dependencies operator=\"And\">");
        for _ in 0..nest_levels {
            xml.push_str("<dependencies operator=\"And\">");
        }
        xml.push_str("<flagDependency flag=\"deep\" value=\"v\"/>");
        for _ in 0..nest_levels {
            xml.push_str("</dependencies>");
        }
        xml.push_str("</dependencies></moduleDependencies></config>");

        let mut cond = compile_from(&xml);
        // Walk down to the node compiled at depth 33.
        for depth in 1..=nest_levels {
            assert_eq!(cond.children.len(), 1, "single chain at depth {depth}");
            cond = cond.children.into_iter().next().unwrap();
        }
        assert_eq!(cond.r#type, FomodConditionType::Composite);
        assert_eq!(cond.op, FomodConditionOp::Or, "truncated node is an Or");
        assert!(
            cond.children.is_empty(),
            "flagDependency at depth 34 dropped"
        );
    }

    #[test]
    fn node_at_exactly_max_depth_still_compiles() {
        // A chain whose deepest node sits at depth 32 is fully compiled.
        let nest_levels = MAX_DEPENDENCY_DEPTH as usize; // node at depth 32
        let mut xml = String::from("<config><moduleDependencies><dependencies operator=\"And\">");
        for _ in 0..nest_levels {
            xml.push_str("<dependencies operator=\"And\">");
        }
        xml.push_str("<flagDependency flag=\"deep\" value=\"v\"/>");
        for _ in 0..nest_levels {
            xml.push_str("</dependencies>");
        }
        xml.push_str("</dependencies></moduleDependencies></config>");

        let mut cond = compile_from(&xml);
        for _ in 1..=nest_levels {
            cond = cond.children.into_iter().next().unwrap();
        }
        assert_eq!(cond.op, FomodConditionOp::And);
        assert_eq!(cond.children.len(), 1);
        assert_eq!(cond.children[0].flag_name, "deep");
    }

    #[test]
    fn more_than_10000_children_are_truncated() {
        let mut xml = String::from("<config><moduleDependencies><dependencies>");
        for i in 0..10005 {
            xml.push_str(&format!("<flagDependency flag=\"f{i}\" value=\"v\"/>"));
        }
        xml.push_str("</dependencies></moduleDependencies></config>");
        let cond = compile_from(&xml);
        assert_eq!(cond.children.len(), 10000);
        assert_eq!(cond.children[9999].flag_name, "f9999");
    }

    #[test]
    fn unknown_elements_still_count_toward_the_children_cap() {
        // The counter increments before the name dispatch, so skipped unknown
        // elements consume cap slots.
        let mut xml = String::from("<config><moduleDependencies><dependencies>");
        for _ in 0..9999 {
            xml.push_str("<unknownElement/>");
        }
        for i in 0..5 {
            xml.push_str(&format!("<flagDependency flag=\"f{i}\" value=\"v\"/>"));
        }
        xml.push_str("</dependencies></moduleDependencies></config>");
        let cond = compile_from(&xml);
        // Slots 1..=9999 unknown (skipped), slot 10000 = f0, then truncated.
        assert_eq!(cond.children.len(), 1);
        assert_eq!(cond.children[0].flag_name, "f0");
    }

    // --- parse_file_entry ---

    fn required_entries(files_xml: &str, prefix: &str) -> Vec<FomodFileEntry> {
        let xml =
            format!("<config><requiredInstallFiles>{files_xml}</requiredInstallFiles></config>");
        parse_str_with_prefix(&xml, prefix).required_files
    }

    #[test]
    fn destination_present_but_empty_differs_from_absent() {
        // <file destination=""> falls back to the source filename through
        // resolve_file_destination; an absent destination falls back to the
        // full source path.
        let entries = required_entries(
            "<file source=\"Dir/Plugin.esp\" destination=\"\"/>\
             <file source=\"Dir/Plugin.esp\"/>",
            "",
        );
        assert_eq!(entries[0].destination, "plugin.esp");
        assert_eq!(entries[1].destination, "dir/plugin.esp");

        // For folders the raw destination is normalized directly: empty stays
        // empty (mod root), absent falls back to the source path.
        let entries = required_entries(
            "<folder source=\"Some/Folder\" destination=\"\"/>\
             <folder source=\"Some/Folder\"/>",
            "",
        );
        assert_eq!(entries[0].destination, "");
        assert_eq!(entries[1].destination, "some/folder");
    }

    #[test]
    fn is_folder_reflects_element_name() {
        let entries = required_entries("<file source=\"a.esp\"/><folder source=\"dir\"/>", "");
        assert!(!entries[0].is_folder);
        assert!(entries[1].is_folder);
    }

    #[test]
    fn source_gets_archive_prefix_then_normalization() {
        let entries = required_entries("<file source=\"Sub\\File.DDS\"/>", "Pre Fix");
        assert_eq!(entries[0].source, "pre fix/sub/file.dds");
        // Empty prefix: no separator is prepended.
        let entries = required_entries("<file source=\"Sub\\File.DDS\"/>", "");
        assert_eq!(entries[0].source, "sub/file.dds");
    }

    #[test]
    fn file_destination_with_trailing_slash_appends_source_filename() {
        let entries = required_entries(
            "<file source=\"a/b/Plugin.esp\" destination=\"Dest/\"/>",
            "",
        );
        assert_eq!(entries[0].destination, "dest/plugin.esp");
    }

    #[test]
    fn priority_follows_pugixml_as_int_semantics() {
        let entries = required_entries(
            "<file source=\"a\" priority=\"12abc\"/>\
             <file source=\"b\" priority=\"abc\"/>\
             <file source=\"c\"/>\
             <file source=\"d\" priority=\"  42\"/>\
             <file source=\"e\" priority=\"-7\"/>\
             <file source=\"f\" priority=\"+3\"/>\
             <file source=\"g\" priority=\"0x1A\"/>\
             <file source=\"h\" priority=\"99999999999\"/>\
             <file source=\"i\" priority=\"-99999999999\"/>\
             <file source=\"j\" priority=\"2147483647\"/>\
             <file source=\"k\" priority=\"2147483648\"/>\
             <file source=\"l\" priority=\"-2147483648\"/>",
            "",
        );
        let priorities: Vec<i32> = entries.iter().map(|e| e.priority).collect();
        assert_eq!(
            priorities,
            vec![
                12,       // strtol-style: digits until first non-digit
                0,        // no digits -> 0
                0,        // missing attribute -> default 0
                42,       // leading ct_space skipped
                -7,       // optional sign
                3,        // optional plus sign
                26,       // pugixml 1.15 as_int accepts 0x hex
                i32::MAX, // decimal overflow saturates high
                i32::MIN, // negative overflow saturates low
                i32::MAX, // exact INT_MAX
                i32::MAX, // INT_MAX + 1 saturates
                i32::MIN, // exact INT_MIN
            ]
        );
    }

    #[test]
    fn bool_attributes_follow_xml_bool_semantics() {
        let entries = required_entries(
            "<file source=\"a\" alwaysInstall=\"true\" installIfUsable=\"1\"/>\
             <file source=\"b\" alwaysInstall=\"TRUE\"/>\
             <file source=\"c\" alwaysInstall=\"false\" installIfUsable=\"0\"/>\
             <file source=\"d\"/>",
            "",
        );
        assert!(entries[0].always_install && entries[0].install_if_usable);
        assert!(entries[1].always_install && !entries[1].install_if_usable);
        assert!(!entries[2].always_install && !entries[2].install_if_usable);
        assert!(!entries[3].always_install && !entries[3].install_if_usable);
    }

    // --- file/folder node filter ---

    #[test]
    fn file_nodes_require_file_or_folder_name_and_non_empty_source() {
        let entries = required_entries(
            "<file/>\
             <file source=\"\"/>\
             <other source=\"nope\"/>\
             <file source=\"ok.esp\"/>\
             <folder source=\"okdir\"/>\
             text-in-between",
            "",
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source, "ok.esp");
        assert_eq!(entries[1].source, "okdir");
    }

    // --- ordering, read from the order attribute on the parent ---

    fn steps_xml(order_attr: &str) -> String {
        let order = if order_attr.is_empty() {
            String::new()
        } else {
            format!(" order=\"{order_attr}\"")
        };
        format!(
            "<config><installSteps{order}>\
             <installStep name=\"Charlie\"><optionalFileGroups/></installStep>\
             <installStep name=\"Alice\"><optionalFileGroups/></installStep>\
             <installStep name=\"Bob\"><optionalFileGroups/></installStep>\
             </installSteps></config>"
        )
    }

    fn step_names(installer: &FomodInstaller) -> Vec<&str> {
        installer.steps.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn step_order_explicit_keeps_document_order() {
        let installer = parse_str(&steps_xml("Explicit"));
        assert_eq!(step_names(&installer), ["Charlie", "Alice", "Bob"]);
    }

    #[test]
    fn step_order_ascending_sorts_by_name() {
        let installer = parse_str(&steps_xml("Ascending"));
        assert_eq!(step_names(&installer), ["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn step_order_descending_sorts_by_name_reversed() {
        let installer = parse_str(&steps_xml("Descending"));
        assert_eq!(step_names(&installer), ["Charlie", "Bob", "Alice"]);
    }

    #[test]
    fn step_order_missing_defaults_to_ascending() {
        let installer = parse_str(&steps_xml(""));
        assert_eq!(step_names(&installer), ["Alice", "Bob", "Charlie"]);
    }

    #[test]
    fn step_order_garbage_falls_through_to_document_order() {
        // Only the exact values "Ascending" and "Descending" sort; anything
        // else, casing mismatches included, keeps document order.
        let installer = parse_str(&steps_xml("ascending"));
        assert_eq!(step_names(&installer), ["Charlie", "Alice", "Bob"]);
        let installer = parse_str(&steps_xml("garbage"));
        assert_eq!(step_names(&installer), ["Charlie", "Alice", "Bob"]);
    }

    #[test]
    fn group_and_plugin_ordering_read_their_own_parents() {
        let installer = parse_str(
            "<config><installSteps order=\"Explicit\">\
             <installStep name=\"S\"><optionalFileGroups order=\"Descending\">\
             <group name=\"A\" type=\"SelectAny\"><plugins order=\"Ascending\">\
             <plugin name=\"z\"/><plugin name=\"a\"/></plugins></group>\
             <group name=\"B\" type=\"SelectAny\"><plugins order=\"Explicit\">\
             <plugin name=\"z\"/><plugin name=\"a\"/></plugins></group>\
             </optionalFileGroups></installStep></installSteps></config>",
        );
        let groups = &installer.steps[0].groups;
        assert_eq!(groups[0].name, "B", "groups sorted Descending");
        assert_eq!(groups[1].name, "A");
        let b_plugins: Vec<&str> = groups[0].plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            b_plugins,
            ["z", "a"],
            "Explicit plugins keep document order"
        );
        let a_plugins: Vec<&str> = groups[1].plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(a_plugins, ["a", "z"], "Ascending plugins sorted by name");
    }

    // --- step ordinal and visibility ---

    #[test]
    fn ordinals_are_assigned_after_ordering() {
        let installer = parse_str(&steps_xml("Ascending"));
        let ordinals: Vec<(String, i32)> = installer
            .steps
            .iter()
            .map(|s| (s.name.clone(), s.ordinal))
            .collect();
        assert_eq!(
            ordinals,
            vec![
                ("Alice".to_string(), 0),
                ("Bob".to_string(), 1),
                ("Charlie".to_string(), 2)
            ]
        );
    }

    fn step_with_visible(visible_inner: &str) -> FomodStep {
        let xml = format!(
            "<config><installSteps order=\"Explicit\"><installStep name=\"S\">\
             <visible>{visible_inner}</visible>\
             <optionalFileGroups/></installStep></installSteps></config>"
        );
        parse_str(&xml).steps.into_iter().next().unwrap()
    }

    #[test]
    fn visible_prefers_dependencies_child() {
        let step = step_with_visible(
            "<dependencies operator=\"Or\"><flagDependency flag=\"f\" value=\"v\"/></dependencies>",
        );
        let vis = step.visible.expect("visible present");
        assert_eq!(vis.op, FomodConditionOp::Or);
        assert_eq!(vis.children.len(), 1);
    }

    #[test]
    fn visible_compiles_from_itself_when_no_dependencies_child() {
        let step = step_with_visible("<flagDependency flag=\"f\" value=\"v\"/>");
        let vis = step.visible.expect("visible present");
        assert_eq!(vis.op, FomodConditionOp::And);
        assert_eq!(vis.children.len(), 1);
        assert_eq!(vis.children[0].flag_name, "f");
    }

    #[test]
    fn empty_or_whitespace_visible_stays_absent() {
        assert!(step_with_visible("").visible.is_none());
        assert!(step_with_visible("\n\t ").visible.is_none());
    }

    // --- typeDescriptor ---

    fn plugin_from(plugin_inner: &str) -> FomodPlugin {
        let xml = format!(
            "<config><installSteps order=\"Explicit\"><installStep name=\"S\">\
             <optionalFileGroups order=\"Explicit\"><group name=\"G\" type=\"SelectAny\">\
             <plugins order=\"Explicit\"><plugin name=\"P\">{plugin_inner}</plugin></plugins>\
             </group></optionalFileGroups></installStep></installSteps></config>"
        );
        parse_str(&xml)
            .steps
            .remove(0)
            .groups
            .remove(0)
            .plugins
            .remove(0)
    }

    #[test]
    fn type_descriptor_type_node_wins() {
        let plugin = plugin_from("<typeDescriptor><type name=\"Required\"/></typeDescriptor>");
        assert_eq!(plugin.r#type, PluginType::Required);
        assert!(plugin.type_patterns.is_empty());
    }

    #[test]
    fn type_node_missing_name_defaults_to_optional() {
        let plugin = plugin_from("<typeDescriptor><type/></typeDescriptor>");
        assert_eq!(plugin.r#type, PluginType::Optional);
    }

    #[test]
    fn no_type_descriptor_leaves_optional() {
        let plugin = plugin_from("");
        assert_eq!(plugin.r#type, PluginType::Optional);
    }

    #[test]
    fn dependency_type_default_type_and_patterns() {
        let plugin = plugin_from(
            "<typeDescriptor><dependencyType>\
             <defaultType name=\"NotUsable\"/>\
             <patterns>\
             <pattern><dependencies operator=\"Or\">\
             <fileDependency file=\"x.esp\"/></dependencies>\
             <type name=\"Recommended\"/></pattern>\
             <pattern><type name=\"CouldBeUsable\"/></pattern>\
             <pattern><dependencies><flagDependency flag=\"f\" value=\"v\"/></dependencies></pattern>\
             </patterns>\
             </dependencyType></typeDescriptor>",
        );
        assert_eq!(plugin.r#type, PluginType::NotUsable);
        assert_eq!(plugin.type_patterns.len(), 3);

        let p0 = &plugin.type_patterns[0];
        assert_eq!(p0.result_type, PluginType::Recommended);
        assert_eq!(p0.condition.op, FomodConditionOp::Or);
        assert_eq!(p0.condition.children.len(), 1);

        // Absent dependencies leave the default condition: Composite/And
        // with no children, which is always true.
        let p1 = &plugin.type_patterns[1];
        assert_eq!(p1.result_type, PluginType::CouldBeUsable);
        assert_eq!(p1.condition, FomodCondition::default());

        // Absent type node leaves result_type Optional.
        let p2 = &plugin.type_patterns[2];
        assert_eq!(p2.result_type, PluginType::Optional);
        assert_eq!(p2.condition.children.len(), 1);
    }

    #[test]
    fn dependency_type_without_default_type_leaves_optional() {
        let plugin = plugin_from(
            "<typeDescriptor><dependencyType><patterns>\
             <pattern><type name=\"Required\"/></pattern>\
             </patterns></dependencyType></typeDescriptor>",
        );
        assert_eq!(plugin.r#type, PluginType::Optional);
        assert_eq!(plugin.type_patterns.len(), 1);
    }

    // --- conditionFlags first-text-node semantics ---

    #[test]
    fn flag_value_is_first_text_node_not_concatenated_mixed_content() {
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">first<middle/>second</flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "first".to_string())]
        );
    }

    #[test]
    fn flag_value_skips_leading_element_children() {
        // pugixml xml_text scans past element children to the first pcdata.
        let plugin =
            plugin_from("<conditionFlags><flag name=\"x\"><middle/>after</flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "after".to_string())]
        );
    }

    #[test]
    fn whitespace_only_flag_value_reads_as_empty() {
        // Whitespace-only pcdata is not in the pugixml tree, so text() = "".
        let plugin = plugin_from("<conditionFlags><flag name=\"x\">\n\t </flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), String::new())]
        );
    }

    #[test]
    fn empty_flag_value_is_kept_and_empty_flag_name_is_skipped() {
        let plugin = plugin_from(
            "<conditionFlags>\
             <flag name=\"kept\"/>\
             <flag name=\"\">value</flag>\
             <flag>value</flag>\
             </conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("kept".to_string(), String::new())]
        );
    }

    #[test]
    fn cdata_flag_values_match_pugixml_first_child_semantics() {
        // Pure CDATA value.
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\"><![CDATA[ raw <value> ]]></flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), " raw <value> ".to_string())]
        );

        // pcdata directly followed by CDATA: pugixml text() returns only the
        // first pcdata child even though roxmltree merges the run.
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">abc<![CDATA[def]]></flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "abc".to_string())]
        );

        // Whitespace pcdata then CDATA: the ws-only pcdata is dropped from
        // the pugixml tree, so the CDATA (kept even when whitespace-only) is
        // the first child.
        let plugin =
            plugin_from("<conditionFlags><flag name=\"x\">  <![CDATA[ ]]></flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), " ".to_string())]
        );

        // CDATA first, then pcdata: the CDATA value wins.
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\"><![CDATA[cd]]>tail</flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "cd".to_string())]
        );
    }

    #[test]
    fn flag_value_entities_are_decoded() {
        let plugin =
            plugin_from("<conditionFlags><flag name=\"x\">a&amp;b&#65;</flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a&bA".to_string())]
        );
        // The rare merged-run path decodes entities in the first chunk too.
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">a&amp;b<![CDATA[cd]]></flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a&b".to_string())]
        );
    }

    // --- conditionalFileInstalls ---

    #[test]
    fn conditional_patterns_default_condition_and_file_filter() {
        let installer = parse_str(
            "<config><conditionalFileInstalls><patterns>\
             <pattern><dependencies operator=\"Or\">\
             <flagDependency flag=\"f\" value=\"v\"/></dependencies>\
             <files><folder source=\"dir\"/><file source=\"\"/><file source=\"a.esp\"/></files>\
             </pattern>\
             <pattern><files><file source=\"b.esp\"/></files></pattern>\
             <pattern/>\
             </patterns></conditionalFileInstalls></config>",
        );
        assert_eq!(installer.conditional_patterns.len(), 3);

        let p0 = &installer.conditional_patterns[0];
        assert_eq!(p0.condition.op, FomodConditionOp::Or);
        assert_eq!(p0.files.len(), 2, "empty-source file node filtered out");

        // Pattern without dependencies: default-constructed condition
        // (Composite/And, no children) = always-true.
        let p1 = &installer.conditional_patterns[1];
        assert_eq!(p1.condition, FomodCondition::default());
        assert_eq!(p1.files.len(), 1);

        let p2 = &installer.conditional_patterns[2];
        assert_eq!(p2.condition, FomodCondition::default());
        assert!(p2.files.is_empty());
    }

    #[test]
    fn conditional_file_installs_without_patterns_yields_none() {
        let installer = parse_str("<config><conditionalFileInstalls/></config>");
        assert!(installer.conditional_patterns.is_empty());
    }

    // --- plugin dependencies ---

    #[test]
    fn plugin_dependencies_are_compiled_when_present() {
        let plugin = plugin_from(
            "<dependencies operator=\"Or\"><flagDependency flag=\"f\" value=\"v\"/></dependencies>",
        );
        let deps = plugin.dependencies.expect("dependencies present");
        assert_eq!(deps.op, FomodConditionOp::Or);
        assert_eq!(deps.children.len(), 1);

        let plugin = plugin_from("");
        assert!(plugin.dependencies.is_none());
    }

    // --- encoding handling ---

    const ENC_SAMPLE: &str = "<config><installSteps order=\"Explicit\">\
        <installStep name=\"Ünïcode Step\"><optionalFileGroups order=\"Explicit\">\
        <group name=\"G\" type=\"SelectAll\"><plugins order=\"Explicit\">\
        <plugin name=\"P\"/></plugins></group>\
        </optionalFileGroups></installStep></installSteps></config>";

    fn utf16_bytes(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let units = text.encode_utf16();
        let head: Box<dyn Iterator<Item = u16>> = if bom {
            Box::new(std::iter::once(0xfeffu16).chain(units))
        } else {
            Box::new(units)
        };
        for unit in head {
            if big_endian {
                out.extend_from_slice(&unit.to_be_bytes());
            } else {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        }
        out
    }

    fn utf32_bytes(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let chars = text.chars().map(|c| c as u32);
        let head: Box<dyn Iterator<Item = u32>> = if bom {
            Box::new(std::iter::once(0xfeffu32).chain(chars))
        } else {
            Box::new(chars)
        };
        for unit in head {
            if big_endian {
                out.extend_from_slice(&unit.to_be_bytes());
            } else {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn all_encodings_of_the_same_document_parse_identically() {
        let reference = parse_module_config(ENC_SAMPLE.as_bytes(), "").unwrap();
        assert_eq!(reference.steps[0].name, "Ünïcode Step");

        let mut utf8_bom = vec![0xef, 0xbb, 0xbf];
        utf8_bom.extend_from_slice(ENC_SAMPLE.as_bytes());

        let variants: Vec<(&str, Vec<u8>)> = vec![
            ("utf8-bom", utf8_bom),
            ("utf16le-bom", utf16_bytes(ENC_SAMPLE, false, true)),
            ("utf16be-bom", utf16_bytes(ENC_SAMPLE, true, true)),
            // BOM-less UTF-16: pugixml guesses from the 3C 00 / 00 3C '<'
            // pattern in the first bytes.
            ("utf16le-bomless", utf16_bytes(ENC_SAMPLE, false, false)),
            ("utf16be-bomless", utf16_bytes(ENC_SAMPLE, true, false)),
            ("utf32le-bom", utf32_bytes(ENC_SAMPLE, false, true)),
            ("utf32be-bom", utf32_bytes(ENC_SAMPLE, true, true)),
        ];
        for (label, bytes) in variants {
            let parsed = parse_module_config(&bytes, "")
                .unwrap_or_else(|e| panic!("{label} failed to parse: {e}"));
            assert_eq!(parsed, reference, "{label} must parse identically to UTF-8");
        }
    }

    #[test]
    fn guess_matches_pugixml_probe_table() {
        assert_eq!(guess_buffer_encoding(b"<config/>"), XmlEncoding::Utf8);
        assert_eq!(
            guess_buffer_encoding(b"\xef\xbb\xbf<c/>"),
            XmlEncoding::Utf8
        );
        assert_eq!(
            guess_buffer_encoding(b"\xff\xfe<\x00"),
            XmlEncoding::Utf16Le
        );
        assert_eq!(
            guess_buffer_encoding(b"\xfe\xff\x00<"),
            XmlEncoding::Utf16Be
        );
        assert_eq!(
            guess_buffer_encoding(b"\xff\xfe\x00\x00"),
            XmlEncoding::Utf32Le,
            "UTF-32 LE BOM outranks the UTF-16 LE BOM prefix"
        );
        assert_eq!(
            guess_buffer_encoding(b"\x00\x00\xfe\xff"),
            XmlEncoding::Utf32Be
        );
        assert_eq!(guess_buffer_encoding(b"<\x00c\x00"), XmlEncoding::Utf16Le);
        assert_eq!(guess_buffer_encoding(b"\x00<\x00c"), XmlEncoding::Utf16Be);
        assert_eq!(
            guess_buffer_encoding(b"<\x00\x00\x00"),
            XmlEncoding::Utf32Le
        );
        assert_eq!(
            guess_buffer_encoding(b"\x00\x00\x00<"),
            XmlEncoding::Utf32Be
        );
        // Too small: detection skipped entirely.
        assert_eq!(guess_buffer_encoding(b"\xff\xfe"), XmlEncoding::Utf8);
        // Declaration probes.
        assert_eq!(
            guess_buffer_encoding(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><c/>"),
            XmlEncoding::Latin1
        );
        assert_eq!(
            guess_buffer_encoding(b"<?xml version=\"1.0\" encoding='latin1'?><c/>"),
            XmlEncoding::Latin1
        );
        assert_eq!(
            guess_buffer_encoding(b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><c/>"),
            XmlEncoding::Utf8,
            "unrecognized declared encodings fall through to UTF-8"
        );
    }

    #[test]
    fn latin1_documents_decode_byte_to_code_point() {
        let mut bytes =
            b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><config><installSteps order=\"Explicit\"><installStep name=\"".to_vec();
        bytes.push(0xdc); // U+00DC LATIN CAPITAL LETTER U WITH DIAERESIS
        bytes.extend_from_slice(b"\"><optionalFileGroups/></installStep></installSteps></config>");
        let installer = parse_module_config(&bytes, "").unwrap();
        assert_eq!(installer.steps[0].name, "\u{dc}");
    }

    #[test]
    fn bom_scalar_is_stripped_before_parsing() {
        // A decoded UTF-16 document must not present U+FEFF to the parser.
        let bytes = utf16_bytes("<config/>", false, true);
        let text = decode_xml_bytes(&bytes).unwrap();
        assert!(text.starts_with("<config"));
    }

    #[test]
    fn invalid_utf8_is_a_decode_error() {
        let err = parse_module_config(b"<config>\xff\xfe</config>", "").unwrap_err();
        assert!(matches!(err, FomodXmlError::Decode(_)));
    }

    #[test]
    fn malformed_xml_is_a_parse_error() {
        let err = parse_module_config(b"<config><unclosed></config>", "").unwrap_err();
        assert!(matches!(err, FomodXmlError::Parse(_)));
    }

    // --- element nesting depth guard ---
    //
    // roxmltree 0.21.1 recurses per nesting level inside Document::parse and
    // aborts the process with an uncatchable stack overflow (measured: debug
    // parses total depth 65, dies at 81, on a 1 MiB stack). load_document
    // bounds the depth before roxmltree runs; see MAX_ELEMENT_DEPTH.

    /// `<config>` (depth 1) wrapping `total_depth - 1` nested `<d>` elements.
    fn deeply_nested_doc(total_depth: usize) -> String {
        let inner = total_depth - 1;
        let mut xml = String::from("<config>");
        for _ in 0..inner {
            xml.push_str("<d>");
        }
        for _ in 0..inner {
            xml.push_str("</d>");
        }
        xml.push_str("</config>");
        xml
    }

    #[test]
    fn element_depth_at_the_limit_still_parses() {
        // Drives real roxmltree recursion at the bound in this test's build
        // profile, so a passing run is evidence the bound is safe with margin
        // (debug overflow starts around depth 81).
        let installer = parse_str(&deeply_nested_doc(MAX_ELEMENT_DEPTH));
        assert_eq!(installer, FomodInstaller::default());
    }

    #[test]
    fn element_depth_beyond_the_limit_is_an_error_not_a_stack_overflow() {
        let err = parse_module_config(deeply_nested_doc(MAX_ELEMENT_DEPTH + 1).as_bytes(), "")
            .unwrap_err();
        assert!(matches!(err, FomodXmlError::TooDeep(MAX_ELEMENT_DEPTH)));
        // A ~14 KB document of this shape killed the whole process inside
        // roxmltree::Document::parse before the guard existed.
        let err = parse_module_config(deeply_nested_doc(2000).as_bytes(), "").unwrap_err();
        assert!(matches!(err, FomodXmlError::TooDeep(_)));
    }

    #[test]
    fn depth_guard_counts_only_real_element_nesting() {
        // Comment, CDATA and PI bodies and the DOCTYPE internal subset may
        // contain `<d>` lookalikes, quoted attribute values may contain raw
        // `>` and `/>`, and siblings and self-closing elements add no depth.
        // None of it may trip the guard on a document whose real nesting is
        // legal.
        let mut xml = String::from(
            "<!DOCTYPE config [<!ENTITY d \"x\">]>\
             <config a=\"x>y/>\"><installSteps order=\"Explicit\">\
             <installStep name=\"a &gt; b > c\"><optionalFileGroups/></installStep>\
             </installSteps>",
        );
        for _ in 0..=MAX_ELEMENT_DEPTH {
            xml.push_str("<!-- <d><d> --><?pi <d> ?><s b=\"1>2\"><![CDATA[<d><d>]]></s><e/>");
        }
        // Real nesting up to exactly the limit (config is depth 1).
        for _ in 0..MAX_ELEMENT_DEPTH - 1 {
            xml.push_str("<d c=\"deep>er\">");
        }
        for _ in 0..MAX_ELEMENT_DEPTH - 1 {
            xml.push_str("</d>");
        }
        xml.push_str("</config>");
        let installer = parse_str(&xml);
        assert_eq!(installer.steps[0].name, "a > b > c");
    }

    // --- character-reference whitespace: pcdata existence on raw chars ---
    //
    // pugixml drops a whitespace-only pcdata run based on the raw source chars
    // before escape expansion, because PUGI_IMPL_SKIPWS stops at the '&' of a
    // character reference. So raw `&#32;` is a pcdata node whose decoded value
    // is " ".

    #[test]
    fn charref_whitespace_flag_value_is_a_real_pcdata_node() {
        let plugin = plugin_from("<conditionFlags><flag name=\"x\">&#32;</flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), " ".to_string())]
        );
        // FOMOD Creation Tool emits &#13;&#10; character references; neither
        // side EOL-normalizes reference-produced CR.
        let plugin =
            plugin_from("<conditionFlags><flag name=\"x\">&#13;&#10;</flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "\r\n".to_string())]
        );
    }

    #[test]
    fn charref_whitespace_module_dependencies_is_present_but_childless() {
        let installer =
            parse_str("<config><moduleDependencies>&#32;</moduleDependencies></config>");
        let deps = installer.module_dependencies.expect("present");
        assert_eq!(deps.r#type, FomodConditionType::Composite);
        assert_eq!(deps.op, FomodConditionOp::And);
        assert!(deps.children.is_empty());
    }

    #[test]
    fn charref_whitespace_visible_is_present_but_childless() {
        let step = step_with_visible("&#32;");
        let vis = step.visible.expect("present");
        assert_eq!(vis.op, FomodConditionOp::And);
        assert!(vis.children.is_empty());
    }

    #[test]
    fn charref_whitespace_survives_in_merged_pcdata_cdata_runs() {
        // The raw first chunk "&#32;" is not whitespace-only, so it is the
        // pcdata node text() returns; its value decodes to " ".
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">&#32;<![CDATA[cd]]></flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), " ".to_string())]
        );
        // Reference-produced CR is not EOL-normalized on the merged-run path
        // either (pugixml decodes the escape after its raw EOL handling).
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">a&#13;<![CDATA[cd]]></flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a\r".to_string())]
        );
    }

    // --- lenient pre-pass: input pugixml accepts must build the same IR
    //     here ---

    #[test]
    fn bare_ampersand_in_attribute_stays_literal() {
        // pugixml loads this and carries the raw name through to the IR.
        let installer = parse_str(
            "<config><installSteps order=\"Explicit\">\
             <installStep name=\"Body & Soul\"><optionalFileGroups/></installStep>\
             </installSteps></config>",
        );
        assert_eq!(installer.steps[0].name, "Body & Soul");
    }

    #[test]
    fn unknown_entity_reference_stays_literal() {
        // pugixml's escape decoder cancels on '&nbsp;' and leaves it as-is.
        let plugin =
            plugin_from("<conditionFlags><flag name=\"x\">a&nbsp;b</flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a&nbsp;b".to_string())]
        );
    }

    #[test]
    fn mixed_valid_and_invalid_references_decode_like_pugixml() {
        // Valid references decode; anything the two parsers disagree on
        // stays literal exactly as pugixml leaves it: uppercase hex prefix,
        // empty/malformed digits, missing semicolon, XML-invalid code points.
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">\
             Fish & Chips &amp; &#x41;&#66; &#X41; &#; &#65a; &lt &copy; &#1; &#xD800;\
             </flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![(
                "x".to_string(),
                "Fish & Chips & AB &#X41; &#; &#65a; &lt &copy; &#1; &#xD800;".to_string()
            )]
        );
    }

    #[test]
    fn dtd_declared_entities_stay_literal_like_pugixml() {
        // pugixml skips the DOCTYPE entirely, so '&foo;' cancels in its
        // escape decoder and stays literal; roxmltree would expand it, but
        // the lenient pass neutralizes the reference first.
        let installer = parse_str(
            "<!DOCTYPE config [<!ENTITY foo \"bar\">]>\
             <config><installSteps order=\"Explicit\"><installStep name=\"S\">\
             <optionalFileGroups order=\"Explicit\"><group name=\"G\" type=\"SelectAny\">\
             <plugins order=\"Explicit\"><plugin name=\"P\">\
             <conditionFlags><flag name=\"x\">&foo;</flag></conditionFlags>\
             </plugin></plugins></group></optionalFileGroups>\
             </installStep></installSteps></config>",
        );
        let plugin = &installer.steps[0].groups[0].plugins[0];
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "&foo;".to_string())]
        );
    }

    #[test]
    fn double_hyphen_comments_are_tolerated() {
        // pugixml only scans for the first '-->'; the body is unobservable
        // (comments are not in the tree with default options).
        let installer = parse_str(
            "<!-- bad -- comment --><config><installSteps order=\"Explicit\">\
             <installStep name=\"A\"><optionalFileGroups/></installStep>\
             </installSteps></config>",
        );
        assert_eq!(installer.steps.len(), 1);
        assert_eq!(installer.steps[0].name, "A");
        // Trailing '-' in the body and a comment between elements.
        let installer = parse_str(
            "<config><installSteps order=\"Explicit\"><!-- dash- -->\
             <installStep name=\"A\"><optionalFileGroups/></installStep>\
             </installSteps></config>",
        );
        assert_eq!(installer.steps[0].name, "A");
    }

    #[test]
    fn cdata_and_comment_interiors_are_not_escaped() {
        // '&' inside CDATA is literal data on both sides and must not be
        // rewritten; '&' inside comments is not entity-parsed by either.
        let plugin = plugin_from(
            "<!-- keep & raw --><conditionFlags>\
             <flag name=\"x\"><![CDATA[a & b &amp; c]]></flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a & b &amp; c".to_string())]
        );
    }

    #[test]
    fn doctype_internal_subset_is_copied_verbatim() {
        // '&' and quotes inside the internal subset must not be rewritten.
        let installer = parse_str(
            "<!DOCTYPE config [<!ENTITY foo \"b&#97;r\">]>\
             <config><installSteps order=\"Explicit\">\
             <installStep name=\"A\"><optionalFileGroups/></installStep>\
             </installSteps></config>",
        );
        assert_eq!(installer.steps[0].name, "A");
    }

    #[test]
    fn stray_cdata_terminator_stays_literal_like_pugixml() {
        // pugixml's pcdata scanner (strconv_pcdata) stops only at '<', '&'
        // and '\r', so a stray ']]>' is ordinary character data there and
        // reaches the IR; roxmltree forbids it, so the lenient pass rewrites
        // it to ']]&gt;'.
        let plugin = plugin_from("<conditionFlags><flag name=\"x\">a]]>b</flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a]]>b".to_string())]
        );
        // Element content, where only pcdata existence matters.
        let installer =
            parse_str("<config><moduleDependencies>a]]>b</moduleDependencies></config>");
        let deps = installer.module_dependencies.expect("present");
        assert!(deps.children.is_empty());
        // Attribute values allow ']]>' on both sides; the rewrite decodes
        // back to the same value.
        let installer = parse_str(
            "<config><installSteps order=\"Explicit\">\
             <installStep name=\"a]]>b\"><optionalFileGroups/></installStep>\
             </installSteps></config>",
        );
        assert_eq!(installer.steps[0].name, "a]]>b");
        // The terminator of a real CDATA section is untouched.
        let plugin =
            plugin_from("<conditionFlags><flag name=\"x\"><![CDATA[a]]>b</flag></conditionFlags>");
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a".to_string())]
        );
    }

    #[test]
    fn xml_declarations_are_skipped_like_pugixml() {
        // pugixml with default options (parse_declaration and parse_pi both
        // off) skips any '<?...?>' by scanning for the first '?>', with no
        // position or grammar validation (the parse_question skip branch);
        // roxmltree validates declarations. The lenient pass rewrites '<?xml'
        // plus whitespace spans to '<!-- -->'.
        let step_doc = |prolog: &str, epilog: &str| {
            format!(
                "{prolog}<config><installSteps order=\"Explicit\">{epilog}\
                 <installStep name=\"A\"><optionalFileGroups/></installStep>\
                 </installSteps></config>"
            )
        };
        // (a) whitespace before the declaration.
        let installer = parse_str(&step_doc(" \n<?xml version=\"1.0\"?>", ""));
        assert_eq!(installer.steps[0].name, "A");
        // (b) declaration lacking the mandatory 'version' attribute.
        let installer = parse_str(&step_doc("<?xml encoding=\"UTF-8\"?>", ""));
        assert_eq!(installer.steps[0].name, "A");
        // (c) declaration mid-document.
        let installer = parse_str(&step_doc("", "<?xml version=\"1.0\"?>"));
        assert_eq!(installer.steps[0].name, "A");
        // (d) declaration after a leading comment.
        let installer = parse_str(&step_doc("<!-- c --><?xml version=\"1.0\"?>", ""));
        assert_eq!(installer.steps[0].name, "A");
        // (e) mid-text: the placeholder comment splits the pcdata run the way
        // the skipped span did in pugixml, so text() is the first run.
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">a<?xml version=\"1.0\"?>b</flag></conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "a".to_string())]
        );
        // Non-declaration '<?...?>' targets are valid roxmltree PIs and are
        // copied verbatim; like pugixml they never reach the IR.
        let installer = parse_str(&step_doc(
            "<?xml-stylesheet type=\"text/xsl\"?>",
            "<?XML ?>",
        ));
        assert_eq!(installer.steps[0].name, "A");
    }

    #[test]
    fn overflowing_char_refs_stay_literal_not_pugixml_wraparound() {
        // Residual divergence inside the recovered reference class, see
        // PARITY-NOTES.md: pugixml strconv_escape accumulates the code point
        // with unsigned wraparound, so 0x100000041 mod 2^32 = 0x41 decodes to
        // "A" and 4294967341 mod 2^32 = 45 decodes to "-". roxmltree rejects
        // the overflow, so the lenient pass neutralizes the '&' and the
        // literal reference text survives instead.
        let plugin = plugin_from(
            "<conditionFlags><flag name=\"x\">&#x100000041; &#4294967341;</flag>\
             </conditionFlags>",
        );
        assert_eq!(
            plugin.condition_flags,
            vec![("x".to_string(), "&#x100000041; &#4294967341;".to_string())]
        );
    }

    // --- accepted strictness divergences: pugixml parses these and this
    //     parser declines. Pinned so the boundary stays deliberate;
    //     rationale in PARITY-NOTES.md. ---

    #[test]
    fn accepted_divergence_multiple_root_elements_fail() {
        // pugixml has no single-root rule; its doc.child("config") finds the
        // second root and parses the document fully.
        let err = parse_module_config(b"<junk/><config><installSteps/></config>", "").unwrap_err();
        assert!(matches!(err, FomodXmlError::Parse(_)));
    }

    #[test]
    fn accepted_divergence_raw_control_chars_fail() {
        // pugixml passes the raw 0x01 byte through into the value.
        let err = parse_module_config(b"<config>\x01</config>", "").unwrap_err();
        assert!(matches!(err, FomodXmlError::Parse(_)));
    }

    #[test]
    fn accepted_divergence_windows_1252_bytes_fail_decode() {
        // The most likely real-world rejection (PARITY-NOTES.md): a
        // ModuleConfig.xml saved as windows-1252 with byte 0x92, a curly
        // apostrophe, in a plugin name. guess_buffer_encoding recognizes only
        // ISO-8859-1 and latin1, so a declared windows-1252 falls through to
        // UTF-8. pugixml does not validate UTF-8 and would carry the raw 0x92
        // byte through as mojibake; a Rust String cannot hold it, so this
        // parser declines with a Decode error. To measure how often this
        // happens, run scripts/run_harness.py against a live MO2 instance.
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"windows-1252\"?>\
            <config><installSteps order=\"Explicit\"><installStep name=\"S\">\
            <optionalFileGroups order=\"Explicit\"><group name=\"G\" type=\"SelectAny\">\
            <plugins order=\"Explicit\"><plugin name=\"Don"
            .to_vec();
        bytes.push(0x92); // cp1252 RIGHT SINGLE QUOTATION MARK, invalid UTF-8
        bytes.extend_from_slice(
            b"t\"/></plugins></group></optionalFileGroups>\
            </installStep></installSteps></config>",
        );
        let err = parse_module_config(&bytes, "").unwrap_err();
        assert!(matches!(err, FomodXmlError::Decode(_)));
    }

    #[test]
    fn accepted_divergence_undeclared_namespace_prefix_fails() {
        // pugixml is namespace-unaware and ignores the xsi attribute;
        // roxmltree resolves prefixes and fails on the undeclared one.
        let err = parse_module_config(
            b"<config xsi:noNamespaceSchemaLocation=\"x\"><installSteps/></config>",
            "",
        )
        .unwrap_err();
        assert!(matches!(err, FomodXmlError::Parse(_)));
    }

    #[test]
    fn accepted_divergence_raw_lt_in_attribute_fails() {
        // pugixml scans an attribute value to the closing quote, allowing a
        // raw '<'; roxmltree forbids '<' in attribute values.
        let err = parse_module_config(
            b"<config><installSteps order=\"Explicit\">\
              <installStep name=\"a<b\"><optionalFileGroups/></installStep>\
              </installSteps></config>",
            "",
        )
        .unwrap_err();
        assert!(matches!(err, FomodXmlError::Parse(_)));
    }

    #[test]
    fn accepted_divergence_duplicate_attributes_fail() {
        // pugixml keeps both attributes and attribute() returns the first.
        let err = parse_module_config(
            b"<config><requiredInstallFiles><file source=\"x\" source=\"y\"/>\
              </requiredInstallFiles></config>",
            "",
        )
        .unwrap_err();
        assert!(matches!(err, FomodXmlError::Parse(_)));
    }

    #[test]
    fn accepted_divergence_prefixed_elements_match_local_names() {
        // pugixml compares raw qualified names, so it finds no "config" root
        // here and yields a default installer; this parser matches
        // namespace-local names and parses the document. Accepted, because
        // real FOMOD schemas use noNamespaceSchemaLocation and are never
        // prefixed.
        let installer = parse_str(
            "<ns:config xmlns:ns=\"urn:x\"><ns:installSteps order=\"Explicit\">\
             <ns:installStep name=\"A\"><ns:optionalFileGroups/></ns:installStep>\
             </ns:installSteps></ns:config>",
        );
        assert_eq!(installer.steps.len(), 1);
        assert_eq!(installer.steps[0].name, "A");
    }

    // --- pugi_as_int unit coverage beyond the file-entry path ---

    #[test]
    fn pugi_as_int_edge_cases() {
        assert_eq!(pugi_as_int(None), 0);
        assert_eq!(pugi_as_int(Some("")), 0);
        assert_eq!(pugi_as_int(Some("-")), 0);
        assert_eq!(pugi_as_int(Some("+")), 0);
        assert_eq!(pugi_as_int(Some("0x")), 0);
        assert_eq!(pugi_as_int(Some("0xFF")), 255);
        assert_eq!(pugi_as_int(Some("-0x10")), -16);
        assert_eq!(
            pugi_as_int(Some("0x00000000001")),
            1,
            "leading zeros do not overflow"
        );
        assert_eq!(
            pugi_as_int(Some("0x123456789")),
            i32::MAX,
            "9 hex digits overflow"
        );
        assert_eq!(pugi_as_int(Some("0007")), 7);
        assert_eq!(pugi_as_int(Some("4294967295")), i32::MAX);
        assert_eq!(pugi_as_int(Some("-2147483649")), i32::MIN);
        assert_eq!(pugi_as_int(Some("9999999999")), i32::MAX);
        assert_eq!(
            pugi_as_int(Some("3999999999")),
            i32::MAX,
            "10 digits, lead 3, > INT_MAX"
        );
        assert_eq!(pugi_as_int(Some("1000000000")), 1000000000);
    }
}
