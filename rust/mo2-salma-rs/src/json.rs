//! Byte-faithful JSON value model + serializer - hand-written replacement for
//! `nlohmann::json` on the inference output path.
//!
//! Task 10's acceptance bar is BYTE-IDENTICAL output to the C++ DLL's
//! `nlohmann::json::dump(2)` (every golden `expected.json` IS that dump). This
//! module reproduces the exact bytes nlohmann emits, so there is no
//! `serde_json` dependency (and none is allowed).
//!
//! ## What "nlohmann `dump(2)`" means, byte for byte
//!
//! - **Sorted object keys.** `nlohmann::json` is `std::map`-backed (NOT
//!   `ordered_json`), so members serialize in `std::string operator<` order ==
//!   unsigned-byte lexicographic order == Rust `str` `Ord`. [`Value::Object`]
//!   stores a [`BTreeMap`], which iterates in exactly that order (all keys are
//!   ASCII). The C++ builder's INSERT order is irrelevant to the output.
//! - **Pretty layout (indent 2).** Two spaces per nesting level; `'\n'`
//!   newlines; an object member line is `<indent>"key": value` (colon then one
//!   space); an array element line is `<indent>value`; members/elements are
//!   joined with `,\n`; `{`/`[` are immediately followed by `\n`; the closing
//!   `}`/`]` sits on its own line at the PARENT indent. There is NO trailing
//!   newline at the end of the document.
//! - **Empty containers inline.** An empty array renders `[]` and an empty
//!   object `{}` on a single line with no interior whitespace, even in pretty
//!   mode.
//! - **Integers vs doubles are distinct types.** A [`Value::Int`] prints as a
//!   plain decimal (`0`, `2`, `802816`); a [`Value::Double`] always carries a
//!   decimal point (`0.0`, `1.0`, `0.54`). Constructing the right variant per
//!   field is load-bearing: the same numeric zero is `"0"` (a count/size) or
//!   `"0.0"` (a confidence component) depending on the C++ static type.
//! - **String escaping** matches nlohmann's default (`ensure_ascii=false`):
//!   `"` -> `\"`, `\` -> `\\`, the C0 shortcuts `\b \f \n \r \t`, any other
//!   control byte `< 0x20` as `\u00XX` with LOWERCASE hex, and `/` is NOT
//!   escaped. Non-ASCII UTF-8 passes through as raw bytes.
//!
//! ## The float rule
//!
//! nlohmann's `dtoa` and Rust's `f64` `Display` both emit the shortest decimal
//! string that round-trips to the same IEEE-754 double, so for identical bits
//! the digit sequence is identical. The ONE systematic difference is that Rust
//! prints an integer-valued double as `1`/`0` while nlohmann prints `1.0`/`0.0`.
//! [`format_double`] appends `.0` exactly when the shortest string contains no
//! `.`, `e`, or `E` (and the value is finite), reproducing nlohmann.
//!
//! Latent risk (documented, not exercised): for magnitudes outside roughly
//! `[1e-5, 1e16]` nlohmann switches to exponent notation with formatting that
//! this simple rule does not replicate. No confidence value or size in any
//! fixture falls in that range (all confidence values are in `[0, 1]`).

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// An owned JSON value. Mirror of the subset of `nlohmann::json` the inference
/// output path constructs.
///
/// [`Value::Int`] and [`Value::Double`] are deliberately separate variants: the
/// C++ code builds integer JSON (counts, sizes, `schema_version`, timings) and
/// double JSON (every confidence field) as distinct static types, and they
/// serialize differently (`0` vs `0.0`). Objects hold a [`BTreeMap`] so keys
/// serialize in the sorted order `std::map`-backed `nlohmann::json` uses.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer (C++ `int` / `int64_t`); prints without a decimal point.
    Int(i64),
    /// JSON integer above `i64::MAX` (C++ `uint64_t` / nlohmann
    /// `number_unsigned_t`); prints without a decimal point.
    ///
    /// Only [`parse`] ever produces this variant - the assembly path builds
    /// [`Value::Int`] for every counter and size, exactly as the C++ does. It
    /// exists so a cached `meta.ini` blob carrying an integer in
    /// `(i64::MAX, u64::MAX]` round-trips through the Tier-1 emitter with the
    /// same digits nlohmann emits, instead of degrading to a float.
    UInt(u64),
    /// JSON double; always prints with a decimal point for integer values.
    Double(f64),
    /// JSON string.
    Str(String),
    /// JSON array.
    Array(Vec<Value>),
    /// JSON object with sorted keys.
    Object(BTreeMap<String, Value>),
}

impl Value {
    /// Construct an empty object.
    pub fn object() -> Value {
        Value::Object(BTreeMap::new())
    }

    /// Construct an empty array.
    pub fn array() -> Value {
        Value::Array(Vec::new())
    }

    /// Construct a string value.
    pub fn string(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// Insert `key`/`val` into an object value, returning `&mut self` for
    /// chaining. Panics if `self` is not a [`Value::Object`].
    pub fn insert(&mut self, key: impl Into<String>, val: Value) -> &mut Self {
        match self {
            Value::Object(map) => {
                map.insert(key.into(), val);
            }
            other => panic!("insert on non-object {other:?}"),
        }
        self
    }

    /// Append `val` to an array value. Panics if `self` is not a
    /// [`Value::Array`].
    pub fn push(&mut self, val: Value) {
        match self {
            Value::Array(items) => items.push(val),
            other => panic!("push on non-array {other:?}"),
        }
    }

    // --- type introspection (mirrors nlohmann `is_*` predicates) -----------

    /// True if this is a [`Value::Str`].
    pub fn is_string(&self) -> bool {
        matches!(self, Value::Str(_))
    }

    /// True if this is a [`Value::Object`].
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    /// True if this is a [`Value::Array`].
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    /// True if this is a number ([`Value::Int`], [`Value::UInt`] or
    /// [`Value::Double`]), matching nlohmann `is_number`.
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::UInt(_) | Value::Double(_))
    }

    /// True if this is a [`Value::Bool`].
    pub fn is_boolean(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    /// True if this is [`Value::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// True if this value is "empty" the way `nlohmann::json::empty()` is: null,
    /// an empty array, or an empty object. Scalars are never empty.
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Array(items) => items.is_empty(),
            Value::Object(map) => map.is_empty(),
            _ => false,
        }
    }

    // --- accessors ---------------------------------------------------------

    /// Borrow the string payload, or `None` if not a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The integer payload, or `None` if not an integer. A [`Value::UInt`] above
    /// `i64::MAX` does not fit and yields `None`, mirroring nlohmann's
    /// `get<int64_t>` range check.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::UInt(n) => i64::try_from(*n).ok(),
            _ => None,
        }
    }

    /// The numeric payload as `f64` (an [`Value::Int`] / [`Value::UInt`] is
    /// widened), or `None` if not a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Int(n) => Some(*n as f64),
            Value::UInt(n) => Some(*n as f64),
            _ => None,
        }
    }

    /// The boolean payload, or `None` if not a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Look up an object member by key, or `None` if `self` is not an object or
    /// the key is absent.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    /// True if `self` is an object containing `key`.
    pub fn contains(&self, key: &str) -> bool {
        matches!(self, Value::Object(map) if map.contains_key(key))
    }

    /// The array length, or `None` if not an array.
    pub fn array_len(&self) -> Option<usize> {
        match self {
            Value::Array(items) => Some(items.len()),
            _ => None,
        }
    }

    /// Borrow the `i`-th array element, or `None` if not an array or out of
    /// range.
    pub fn get_index(&self, i: usize) -> Option<&Value> {
        match self {
            Value::Array(items) => items.get(i),
            _ => None,
        }
    }

    // --- serialization -----------------------------------------------------

    /// Serialize with `indent` spaces per nesting level, reproducing
    /// `nlohmann::json::dump(indent)` byte for byte. No trailing newline.
    pub fn dump(&self, indent: usize) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, indent, 0);
        out
    }

    fn write_pretty(&self, out: &mut String, indent: usize, depth: usize) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(n) => {
                // i64 Display is plain decimal, matching nlohmann integer output.
                let _ = write!(out, "{n}");
            }
            Value::UInt(n) => {
                // u64 Display is plain decimal, matching nlohmann's
                // number_unsigned_t output.
                let _ = write!(out, "{n}");
            }
            Value::Double(d) => out.push_str(&format_double(*d)),
            Value::Str(s) => write_escaped(out, s),
            Value::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                let child_pad = (depth + 1) * indent;
                let last = items.len() - 1;
                for (i, item) in items.iter().enumerate() {
                    push_spaces(out, child_pad);
                    item.write_pretty(out, indent, depth + 1);
                    if i != last {
                        out.push(',');
                    }
                    out.push('\n');
                }
                push_spaces(out, depth * indent);
                out.push(']');
            }
            Value::Object(map) => {
                if map.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                let child_pad = (depth + 1) * indent;
                let last = map.len() - 1;
                for (i, (key, val)) in map.iter().enumerate() {
                    push_spaces(out, child_pad);
                    write_escaped(out, key);
                    out.push_str(": ");
                    val.write_pretty(out, indent, depth + 1);
                    if i != last {
                        out.push(',');
                    }
                    out.push('\n');
                }
                push_spaces(out, depth * indent);
                out.push('}');
            }
        }
    }
}

fn push_spaces(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

/// Format an `f64` exactly as `nlohmann::json::dump` does: the shortest
/// round-tripping decimal, with `.0` appended for integer-valued finite doubles
/// (which Rust's `Display` would otherwise print as a bare integer). Non-finite
/// values render as `null`, matching nlohmann's default handling of NaN/Inf.
pub fn format_double(v: f64) -> String {
    if !v.is_finite() {
        return "null".to_string();
    }
    // Rust `f64` `Display` is the shortest decimal that round-trips, the same
    // guarantee nlohmann's `dtoa` provides; identical bits give identical
    // digits. The only systematic gap is the missing decimal point on
    // integer-valued doubles.
    let s = format!("{v}");
    if s.bytes().any(|b| b == b'.' || b == b'e' || b == b'E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Parse a JSON document into a [`Value`], the read counterpart of [`Value::dump`].
///
/// Used only on the Tier-1 inference path to decode the cached `fomod-plus` blob
/// stored in `meta.ini`, mirroring the C++ `nlohmann::json::parse(value)` call in
/// `FomodInferenceService::try_fomod_plus_json`. That call is wrapped in a
/// `try/catch(json::parse_error)` that discards the candidate on any failure, so
/// this parser reports errors via `Err(String)` and NEVER panics on malformed
/// input; the caller treats `Err` exactly as the C++ treats a caught parse error.
///
/// The grammar is RFC 8259 as nlohmann implements it, NOT a lenient superset:
/// leading zeros (`01`), a leading `+`, and a bare `.5` / `1.` are rejected;
/// raw control bytes below `0x20` inside a string are rejected; a `\uXXXX`
/// escape must be exactly four hex digits (no sign). Accepting any of these
/// would flip a Tier-1 MISS into a Tier-1 HIT and change the whole output
/// document relative to the C++.
///
/// Numbers follow nlohmann's integer-vs-float split: a token containing `.`, `e`,
/// or `E` becomes a [`Value::Double`]; an integer in `i64` range becomes a
/// [`Value::Int`], one in `(i64::MAX, u64::MAX]` a [`Value::UInt`], and anything
/// wider a [`Value::Double`]. Duplicate object keys keep the last occurrence
/// (nlohmann's behavior); object keys serialize back in sorted order regardless.
/// Trailing non-whitespace content after the top-level value is an error.
///
/// Nesting is capped at [`MAX_PARSE_DEPTH`]; see that constant for why a cap
/// exists at all when nlohmann has none.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        depth: 0,
    };
    let value = parser.value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(format!("trailing content at byte {}", parser.pos));
    }
    Ok(value)
}

/// Maximum container nesting [`parse`] will descend into before failing.
///
/// nlohmann's parser is ITERATIVE (a heap `std::vector<bool> states` stack, and
/// an iterative `destroy()`), so it has no depth limit and simply parses
/// arbitrarily deep input. This parser is recursive descent, so an unbounded
/// document would exhaust the thread stack - and a Windows stack overflow is an
/// SEH exception, NOT a Rust panic, so `capi`'s `catch_unwind` firewall cannot
/// contain it: the HOST process (MO2, or `mo2-server.exe`) would die where the
/// C++ DLL returns a normal document. Measured on this host, a release build
/// survived depth 2000 and died at depth 3000 with `STATUS_STACK_OVERFLOW`.
///
/// The cap turns that crash into an `Err`, which `try_fomod_plus_json` maps to a
/// Tier-1 miss - the same observable outcome the C++ reaches for any such blob,
/// because a real fomod-plus document nests about 5 levels and anything deeper
/// can never name-resolve against the installer. 512 is ~100x the depth a
/// genuine cached blob uses and small enough to be safe on a 1 MiB thread stack.
/// Same guard class as the ported `MAX_ELEMENT_DEPTH` (XML) and
/// `MAX_DEPENDENCY_DEPTH` (condition trees). See PARITY-NOTES "Task 12".
pub const MAX_PARSE_DEPTH: usize = 512;

/// Recursive-descent parser state over the raw UTF-8 bytes of the document.
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// Current container nesting depth, bounded by [`MAX_PARSE_DEPTH`].
    depth: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.bytes.get(self.pos) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(_) => self.number(),
            None => Err("unexpected end of JSON".to_string()),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(format!("invalid literal at byte {}", self.pos))
        }
    }

    /// Enter one container level, failing past [`MAX_PARSE_DEPTH`].
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err(format!(
                "nesting deeper than {MAX_PARSE_DEPTH} at byte {}",
                self.pos
            ));
        }
        Ok(())
    }

    /// Parse an object, accounting one nesting level. Depth is released only on
    /// success; an `Err` aborts the whole parse, so it need not unwind.
    fn object(&mut self) -> Result<Value, String> {
        self.enter()?;
        let value = self.object_body()?;
        self.depth -= 1;
        Ok(value)
    }

    fn object_body(&mut self) -> Result<Value, String> {
        self.pos += 1; // consume '{'
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err(format!("expected object key at byte {}", self.pos));
            }
            let key = self.string()?;
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(format!("expected ':' at byte {}", self.pos));
            }
            self.pos += 1;
            let val = self.value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.pos)),
            }
        }
    }

    /// Parse an array, accounting one nesting level. See [`Parser::object`].
    fn array(&mut self) -> Result<Value, String> {
        self.enter()?;
        let value = self.array_body()?;
        self.depth -= 1;
        Ok(value)
    }

    fn array_body(&mut self) -> Result<Value, String> {
        self.pos += 1; // consume '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.pos)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1; // consume opening '"'
        let mut out = String::new();
        loop {
            match self.bytes.get(self.pos) {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = *self
                        .bytes
                        .get(self.pos)
                        .ok_or_else(|| "truncated escape".to_string())?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        other => return Err(format!("bad escape \\{}", other as char)),
                    }
                }
                // nlohmann rejects a raw control byte inside a string
                // (parse_error 101): it must be escaped. Accepting one here
                // would parse a blob the C++ discards.
                Some(&b) if b < 0x20 => {
                    return Err(format!("raw control byte {b:#04x} in string"));
                }
                Some(&b) => {
                    // Copy one UTF-8 code point verbatim (the source is valid
                    // UTF-8, so the continuation bytes are well-formed).
                    let len = match b {
                        0x00..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    let end = self.pos + len;
                    let slice = self
                        .bytes
                        .get(self.pos..end)
                        .ok_or_else(|| "truncated UTF-8".to_string())?;
                    out.push_str(
                        std::str::from_utf8(slice).map_err(|_| "invalid UTF-8".to_string())?,
                    );
                    self.pos = end;
                }
            }
        }
    }

    /// Decode a `\uXXXX` escape (already past the `u`), combining a surrogate
    /// pair when a high surrogate is followed by `\uXXXX` low surrogate.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let hi = self.hex4()?;
        if (0xd800..=0xdbff).contains(&hi) {
            // High surrogate: require a following low surrogate escape.
            if self.bytes.get(self.pos) == Some(&b'\\')
                && self.bytes.get(self.pos + 1) == Some(&b'u')
            {
                self.pos += 2;
                let lo = self.hex4()?;
                if (0xdc00..=0xdfff).contains(&lo) {
                    let c = 0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00);
                    return char::from_u32(c).ok_or_else(|| "bad surrogate pair".to_string());
                }
            }
            return Err("lone high surrogate".to_string());
        }
        if (0xdc00..=0xdfff).contains(&hi) {
            return Err("lone low surrogate".to_string());
        }
        char::from_u32(hi).ok_or_else(|| "bad code point".to_string())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let slice = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| "truncated \\u escape".to_string())?;
        // Require four ASCII hex digits. `u32::from_str_radix` would also accept
        // a leading '+' (so `\u+123` would decode), which nlohmann rejects.
        let mut code: u32 = 0;
        for &b in slice {
            let digit = match b {
                b'0'..=b'9' => u32::from(b - b'0'),
                b'a'..=b'f' => u32::from(b - b'a') + 10,
                b'A'..=b'F' => u32::from(b - b'A') + 10,
                _ => return Err("bad \\u hex".to_string()),
            };
            code = code * 16 + digit;
        }
        self.pos += 4;
        Ok(code)
    }

    /// Parse a number token under the strict RFC 8259 grammar nlohmann enforces:
    /// `-? (0 | [1-9][0-9]*) ( '.' [0-9]+ )? ( [eE] [+-]? [0-9]+ )?`.
    ///
    /// Scanning a permissive character class and deferring to Rust's `FromStr`
    /// would accept `01`, `+5`, `.5` and `1.` - all of which nlohmann rejects
    /// with parse_error 101. On the Tier-1 path that difference is not cosmetic:
    /// the C++ discards the whole cached blob and runs the full solve, so
    /// accepting it here would emit a completely different document.
    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;

        // Optional minus (a leading '+' is NOT valid JSON).
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }

        // Integer part: a lone '0', or a nonzero digit followed by digits. A
        // leading zero such as `01` is rejected.
        match self.bytes.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(format!("invalid value at byte {start}")),
        }

        let mut is_float = false;

        // Fraction: '.' must be followed by at least one digit (`1.` is invalid).
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                return Err(format!("expected digit after '.' at byte {}", self.pos));
            }
            while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            is_float = true;
        }

        // Exponent: [eE] with an optional sign and at least one digit.
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                return Err(format!("expected digit in exponent at byte {}", self.pos));
            }
            while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            is_float = true;
        }

        // The token is ASCII by construction, so this cannot fail.
        let s = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "bad number".to_string())?;

        // nlohmann's number split: a fractional/exponent token is a double;
        // otherwise int64 if it fits, else uint64, else a double.
        if is_float {
            return s
                .parse::<f64>()
                .map(Value::Double)
                .map_err(|_| format!("bad number {s:?}"));
        }
        if let Ok(i) = s.parse::<i64>() {
            return Ok(Value::Int(i));
        }
        if let Ok(u) = s.parse::<u64>() {
            return Ok(Value::UInt(u));
        }
        s.parse::<f64>()
            .map(Value::Double)
            .map_err(|_| format!("bad number {s:?}"))
    }
}

/// Append `s` as a JSON string literal (surrounding quotes included) with
/// nlohmann's default (`ensure_ascii=false`) escaping.
fn write_escaped(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // Other C0 controls: \u00XX with lowercase hex.
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            // '/' is intentionally NOT escaped; non-ASCII passes through raw.
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- sorted-key emission ----------------------------------------------

    #[test]
    fn object_keys_emit_sorted_regardless_of_insert_order() {
        let mut obj = Value::object();
        obj.insert("zebra", Value::Int(1));
        obj.insert("alpha", Value::Int(2));
        obj.insert("mike", Value::Int(3));
        // Sorted: alpha, mike, zebra.
        assert_eq!(
            obj.dump(2),
            "{\n  \"alpha\": 2,\n  \"mike\": 3,\n  \"zebra\": 1\n}"
        );
    }

    // --- nested indentation + separators ----------------------------------

    #[test]
    fn nested_object_and_array_indentation() {
        let mut inner = Value::object();
        inner.insert("b", Value::Int(2));
        inner.insert("a", Value::Int(1));
        let mut arr = Value::array();
        arr.push(Value::Int(1));
        arr.push(Value::Int(2));
        let mut root = Value::object();
        root.insert("obj", inner);
        root.insert("arr", arr);
        // Keys sorted: arr, obj. ": " member sep, ",\n" element sep.
        let expected = "{\n  \"arr\": [\n    1,\n    2\n  ],\n  \"obj\": {\n    \"a\": 1,\n    \"b\": 2\n  }\n}";
        assert_eq!(root.dump(2), expected);
    }

    // --- empty containers render inline -----------------------------------

    #[test]
    fn empty_array_and_object_render_inline() {
        assert_eq!(Value::array().dump(2), "[]");
        assert_eq!(Value::object().dump(2), "{}");
        let mut root = Value::object();
        root.insert("reasons", Value::array());
        root.insert("meta", Value::object());
        assert_eq!(root.dump(2), "{\n  \"meta\": {},\n  \"reasons\": []\n}");
    }

    // --- the float ".0" rule and Int-vs-Double distinction ----------------

    #[test]
    fn double_integer_values_get_decimal_point() {
        assert_eq!(Value::Double(1.0).dump(2), "1.0");
        assert_eq!(Value::Double(0.0).dump(2), "0.0");
        assert_eq!(Value::Double(0.5).dump(2), "0.5");
        assert_eq!(Value::Double(2.0).dump(2), "2.0");
    }

    #[test]
    fn int_and_double_zero_differ() {
        assert_eq!(Value::Int(0).dump(2), "0");
        assert_eq!(Value::Double(0.0).dump(2), "0.0");
        assert_eq!(Value::Int(802816).dump(2), "802816");
    }

    // --- string escaping ---------------------------------------------------

    #[test]
    fn string_escaping_matches_nlohmann_default() {
        assert_eq!(Value::string("a\"b").dump(2), "\"a\\\"b\"");
        assert_eq!(Value::string("a\\b").dump(2), "\"a\\\\b\"");
        assert_eq!(Value::string("a\nb").dump(2), "\"a\\nb\"");
        assert_eq!(Value::string("a\tb").dump(2), "\"a\\tb\"");
        assert_eq!(Value::string("a\rb").dump(2), "\"a\\rb\"");
        assert_eq!(Value::string("a\u{08}b").dump(2), "\"a\\bb\"");
        assert_eq!(Value::string("a\u{0C}b").dump(2), "\"a\\fb\"");
        // Other control char -> lowercase \u00XX.
        assert_eq!(Value::string("a\u{01}b").dump(2), "\"a\\u0001b\"");
        assert_eq!(Value::string("\u{1f}").dump(2), "\"\\u001f\"");
        // Forward slash is NOT escaped.
        assert_eq!(Value::string("a/b").dump(2), "\"a/b\"");
        // Non-ASCII UTF-8 passes through raw.
        assert_eq!(Value::string("café").dump(2), "\"café\"");
    }

    // --- no trailing newline ----------------------------------------------

    #[test]
    fn no_trailing_newline() {
        let mut root = Value::object();
        root.insert("k", Value::Int(1));
        let s = root.dump(2);
        assert!(s.ends_with('}'));
        assert!(!s.ends_with('\n'));
    }

    // --- FLOAT ORACLE: hard values from the fixture confidence fields ------
    //
    // Each MUST match nlohmann's shortest-round-trip output. They will iff the
    // formatter is Rust `Display` + the ".0" rule, because identical f64 bits
    // give identical shortest digit sequences.

    #[test]
    fn float_oracle_hard_values() {
        let cases: &[(f64, &str)] = &[
            (0.6, "0.6"),
            (0.54, "0.54"),
            (0.58, "0.58"),
            (0.5, "0.5"),
            (0.7, "0.7"),
            (0.3, "0.3"),
            (0.85, "0.85"),
            (0.8950000000000001, "0.8950000000000001"),
            (0.9176215277777777, "0.9176215277777777"),
            (0.9176215277777778, "0.9176215277777778"),
            (0.9999999999999999, "0.9999999999999999"),
            (0.9250000000000002, "0.9250000000000002"),
            (1.0, "1.0"),
            (0.0, "0.0"),
        ];
        for (v, want) in cases {
            assert_eq!(format_double(*v), *want, "format_double({v})");
        }
    }

    #[test]
    fn composite_from_all_ones_is_not_exactly_one() {
        // The exact IEEE-754 sum the C++ `composite_from` computes for a fully
        // forced plugin: 0.40 + 0.30 + 0.20 + 0.10 rounds to 0.9999999999999999,
        // which every golden fixture emits for such plugins.
        let composite = 0.40_f64 * 1.0 + 0.30 * 1.0 + 0.20 * 1.0 + 0.10 * 1.0;
        assert_eq!(format_double(composite), "0.9999999999999999");
    }

    // --- introspection helpers (used by the ported unit tests) ------------

    #[test]
    fn introspection_predicates_and_accessors() {
        assert!(Value::string("x").is_string());
        assert!(Value::object().is_object());
        assert!(Value::array().is_array());
        assert!(Value::Int(1).is_number());
        assert!(Value::Double(1.0).is_number());
        assert!(Value::Bool(true).is_boolean());
        assert!(Value::Null.is_null());

        let mut obj = Value::object();
        obj.insert("name", Value::string("Plugin1"));
        assert!(obj.contains("name"));
        assert!(!obj.contains("missing"));
        assert_eq!(obj.get("name").and_then(Value::as_str), Some("Plugin1"));
        assert_eq!(obj.get("missing"), None);

        assert_eq!(Value::Int(42).as_i64(), Some(42));
        assert_eq!(Value::Double(0.5).as_f64(), Some(0.5));
        assert_eq!(Value::Int(3).as_f64(), Some(3.0));
        assert_eq!(Value::Bool(false).as_bool(), Some(false));
        assert_eq!(Value::string("x").as_i64(), None);
    }

    #[test]
    fn is_empty_matches_nlohmann() {
        assert!(Value::Null.is_empty());
        assert!(Value::array().is_empty());
        assert!(Value::object().is_empty());
        assert!(!Value::Int(0).is_empty());
        assert!(!Value::string("").is_empty());
        let mut a = Value::array();
        a.push(Value::Int(1));
        assert!(!a.is_empty());
    }

    // --- parse (Tier-1 fomod-plus blob decoder) ---------------------------

    #[test]
    fn parse_scalars_and_containers() {
        assert_eq!(parse("null").unwrap(), Value::Null);
        assert_eq!(parse("  true ").unwrap(), Value::Bool(true));
        assert_eq!(parse("false").unwrap(), Value::Bool(false));
        assert_eq!(parse("42").unwrap(), Value::Int(42));
        assert_eq!(parse("-7").unwrap(), Value::Int(-7));
        // Fractional/exponent tokens are doubles; plain integers are ints.
        assert_eq!(parse("0.5").unwrap(), Value::Double(0.5));
        assert_eq!(parse("1e3").unwrap(), Value::Double(1000.0));
        assert_eq!(parse("\"hi\"").unwrap(), Value::string("hi"));
        assert_eq!(parse("[]").unwrap(), Value::array());
        assert_eq!(parse("{}").unwrap(), Value::object());
    }

    #[test]
    fn parse_object_and_array_round_trip() {
        let v = parse(r#"{"steps":[{"name":"Main","groups":[]}]}"#).unwrap();
        let steps = v.get("steps").unwrap();
        assert_eq!(steps.array_len(), Some(1));
        assert_eq!(
            steps
                .get_index(0)
                .unwrap()
                .get("name")
                .and_then(Value::as_str),
            Some("Main")
        );
        // dump re-emits with sorted keys; parsing it back yields an equal value.
        assert_eq!(parse(&v.dump(2)).unwrap(), v);
    }

    #[test]
    fn parse_string_escapes_including_surrogate_pair() {
        assert_eq!(parse(r#""a\"b""#).unwrap(), Value::string("a\"b"));
        assert_eq!(parse(r#""a\\b""#).unwrap(), Value::string("a\\b"));
        assert_eq!(
            parse(r#""line\nfeed""#).unwrap(),
            Value::string("line\nfeed")
        );
        assert_eq!(parse(r#""A""#).unwrap(), Value::string("A"));
        // U+1F600 as a UTF-16 surrogate pair.
        assert_eq!(parse(r#""😀""#).unwrap(), Value::string("\u{1f600}"));
        // Raw non-ASCII UTF-8 passes through.
        assert_eq!(parse("\"café\"").unwrap(), Value::string("café"));
    }

    #[test]
    fn parse_duplicate_keys_keep_last() {
        // nlohmann keeps the last occurrence of a duplicated key.
        let v = parse(r#"{"a":1,"a":2}"#).unwrap();
        assert_eq!(v.get("a").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn parse_rejects_malformed_without_panicking() {
        for bad in [
            "",
            "{",
            "[1,2",
            "{\"a\":}",
            "truer",
            "\"unterminated",
            "{\"a\":1} trailing",
            "01x",
            "\"\\ud83d\"", // lone high surrogate
        ] {
            assert!(parse(bad).is_err(), "expected parse error for {bad:?}");
        }
    }

    #[test]
    fn parse_number_grammar_is_strict_like_nlohmann() {
        // nlohmann rejects each of these with parse_error 101. Accepting them
        // would turn a Tier-1 MISS into a Tier-1 HIT and emit a different
        // document, so leniency here is a real parity bug, not a nicety.
        for bad in [
            "01",         // leading zero
            "-01",        // leading zero after the sign
            "+5",         // leading plus
            ".5",         // no integer part
            "1.",         // no fraction digits
            "1e",         // no exponent digits
            "1e+",        // sign but no exponent digits
            "-",          // sign only
            "{\"a\":01}", // nested, the shape a cached blob would carry
        ] {
            assert!(parse(bad).is_err(), "expected parse error for {bad:?}");
        }

        // ...while the valid forms still parse to the right variant.
        assert_eq!(parse("0").unwrap(), Value::Int(0));
        assert_eq!(parse("-0").unwrap(), Value::Int(0));
        assert_eq!(parse("10").unwrap(), Value::Int(10));
        assert_eq!(parse("1.5").unwrap(), Value::Double(1.5));
        assert_eq!(parse("1e-3").unwrap(), Value::Double(0.001));
        assert_eq!(parse("0.0").unwrap(), Value::Double(0.0));
    }

    #[test]
    fn parse_big_integers_use_uint_and_keep_their_digits() {
        // nlohmann stores an integer above i64::MAX as number_unsigned_t and
        // dumps the exact digits; degrading to a double would print a mangled
        // float. Reachable through the Tier-1 emitter, which echoes a cached
        // `deselected` entry's raw `name` value into the output.
        assert_eq!(parse("9223372036854775807").unwrap(), Value::Int(i64::MAX));
        assert_eq!(
            parse("9223372036854775808").unwrap(),
            Value::UInt(9_223_372_036_854_775_808)
        );
        assert_eq!(
            parse("18446744073709551615").unwrap(),
            Value::UInt(u64::MAX)
        );
        assert_eq!(
            Value::UInt(18_446_744_073_709_551_615).dump(2),
            "18446744073709551615"
        );
        // Wider than u64 falls back to a double, as nlohmann does.
        assert!(matches!(
            parse("18446744073709551616").unwrap(),
            Value::Double(_)
        ));
    }

    #[test]
    fn parse_rejects_raw_control_bytes_and_signed_unicode_escapes() {
        // A raw control byte inside a string is parse_error 101 in nlohmann; it
        // has to be escaped.
        assert!(parse("\"a\tb\"").is_err());
        assert!(parse("\"a\nb\"").is_err());
        assert!(parse("\"a\u{1}b\"").is_err());
        // The escaped forms remain valid.
        assert_eq!(parse(r#""a\tb""#).unwrap(), Value::string("a\tb"));
        assert_eq!(parse(r#""a\u0001b""#).unwrap(), Value::string("a\u{1}b"));
        // `u32::from_str_radix` would accept a sign, so `\u+123` must not decode.
        assert!(parse(r#""\u+123""#).is_err());
        assert!(parse(r#""\u 123""#).is_err());
        assert!(parse(r#""\uzzzz""#).is_err());
    }

    #[test]
    fn parse_depth_is_capped_instead_of_overflowing_the_stack() {
        // At the cap the document still parses...
        let deep_ok = format!(
            "{}1{}",
            "[".repeat(MAX_PARSE_DEPTH),
            "]".repeat(MAX_PARSE_DEPTH)
        );
        assert!(parse(&deep_ok).is_ok());

        // ...one level further is a clean Err, NOT a stack overflow. Without the
        // cap this input class aborts the host process (a Windows stack overflow
        // is an SEH exception that `capi`'s catch_unwind cannot contain).
        let too_deep = format!(
            "{}1{}",
            "[".repeat(MAX_PARSE_DEPTH + 1),
            "]".repeat(MAX_PARSE_DEPTH + 1)
        );
        assert!(parse(&too_deep).is_err());

        // The unterminated form a hostile meta.ini would actually carry.
        assert!(parse(&"[".repeat(100_000)).is_err());

        // Depth is per-path, not cumulative: many shallow siblings are fine.
        let wide = format!("[{}]", vec!["[1]"; 5000].join(","));
        assert!(parse(&wide).is_ok());
    }
}
