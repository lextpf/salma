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
    /// Serialize to a string. `indent > 0` selects nlohmann's pretty layout
    /// (two spaces per level); `indent == 0` is the compact form.
    ///
    /// Delegates to `serde_json`, which was verified byte-identical to
    /// nlohmann's `dump(2)` across the whole committed golden corpus: sorted
    /// keys, two-space pretty layout, empty containers inline, the integer /
    /// double split, lowercase `\u00XX` control escapes, unescaped `/`, raw
    /// non-ASCII passthrough, and no trailing newline all match. See
    /// PARITY-NOTES.
    pub fn dump(&self, indent: usize) -> String {
        let v = to_serde(self);
        if indent == 0 {
            serde_json::to_string(&v).expect("Value cannot fail to serialize")
        } else {
            serde_json::to_string_pretty(&v).expect("Value cannot fail to serialize")
        }
    }
}

/// Convert to `serde_json::Value` for serialization.
///
/// The integer/double split survives the hop: `serde_json::Number` keeps `i64`,
/// `u64` and `f64` distinct, so a [`Value::Int`] still prints `0` while a
/// [`Value::Double`] still prints `0.0`. That distinction is load-bearing - the
/// same numeric zero is a count or a confidence component depending on the C++
/// static type.
///
/// A non-finite double has no JSON representation; nlohmann emits `null` for
/// NaN and the infinities, and `Number::from_f64` returning `None` reproduces
/// that exactly.
fn to_serde(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(n) => serde_json::Value::Number((*n).into()),
        Value::UInt(n) => serde_json::Value::Number((*n).into()),
        Value::Double(d) => serde_json::Number::from_f64(*d)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(to_serde).collect()),
        // serde_json's Map is BTreeMap-backed by default (no `preserve_order`
        // feature), so keys stay in the sorted order nlohmann's std::map emits.
        Value::Object(map) => {
            serde_json::Value::Object(map.iter().map(|(k, v)| (k.clone(), to_serde(v))).collect())
        }
    }
}

/// Convert from `serde_json::Value` after parsing.
fn from_serde(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            // The same split nlohmann makes: an integer token in i64 range is
            // signed, one above it unsigned, anything else a double.
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::UInt(u)
            } else {
                Value::Double(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Array(items) => {
            Value::Array(items.into_iter().map(from_serde).collect())
        }
        serde_json::Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, from_serde(v))).collect())
        }
    }
}

/// Format a double the way nlohmann does.
///
/// Both nlohmann's `dtoa` and Rust's `f64` `Display` emit the shortest decimal
/// that round-trips to the same IEEE-754 double, so for identical bits the
/// digits agree. The ONE systematic difference is that Rust prints an
/// integer-valued double as `1`/`0` where nlohmann prints `1.0`/`0.0`, so a
/// `.0` is appended when the shortest form carries no `.`, `e` or `E`.
///
/// Retained after the serde_json switch: it is the reference the diagnostics
/// tests assert confidence rendering against, independently of the JSON layer.
pub fn format_double(v: f64) -> String {
    let s = format!("{v}");
    if v.is_finite() && !s.contains(['.', 'e', 'E']) {
        format!("{s}.0")
    } else {
        s
    }
}

/// Parse a JSON document. Mirror of the `nlohmann::json::parse(value)` call in
/// `FomodInferenceService::try_fomod_plus_json`, which is wrapped in a
/// `try/catch(json::parse_error)` that discards the candidate on any failure.
/// Errors are reported as `Err(String)` and this NEVER panics on malformed
/// input; the caller treats `Err` exactly as the C++ treats a caught parse
/// error.
///
/// Backed by `serde_json`, which implements the same strict RFC 8259 grammar
/// nlohmann does rather than a lenient superset: leading zeros (`01`), a
/// leading `+`, and a bare `.5` / `1.` are all rejected, as are raw control
/// bytes below `0x20` inside a string. Accepting any of those would flip a
/// Tier-1 MISS into a Tier-1 HIT and change the whole output document relative
/// to the C++. Duplicate object keys keep the last occurrence, matching
/// nlohmann; keys serialize back sorted regardless.
///
/// Two documented divergences from the hand-written parser this replaced,
/// neither reachable from the inputs the engine actually parses (the
/// fomod-plus cache blob and the install selections JSON, which carry only
/// strings, booleans and small integers):
///
/// - **Float precision.** serde_json's number parser is not always
///   correctly-rounded: `0.9999999999999999` parses 1 ULP high, to exactly
///   `1.0`. Serialization is unaffected, so engine-computed doubles still
///   render byte-identically; only re-reading a float from JSON text differs.
/// - **Nesting depth.** serde_json's recursion limit is 128, where this
///   module's own cap was [`MAX_PARSE_DEPTH`]. A document nested deeper than
///   128 now fails to parse instead of succeeding.
pub fn parse(text: &str) -> Result<Value, String> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => Ok(from_serde(v)),
        Err(e) => Err(e.to_string()),
    }
}

/// Nesting depth the hand-written parser capped at, kept as documentation of
/// the previous limit. The effective cap is now serde_json's 128.
pub const MAX_PARSE_DEPTH: usize = 512;

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
        // DIVERGENCE from nlohmann, introduced by the serde_json switch: it
        // reads "-0" as the float -0.0 where nlohmann reads integer 0. Kept as
        // an assertion rather than a fix because it is unreachable from what
        // the engine parses (the fomod-plus blob and the install selections
        // JSON carry names, booleans and small counts), and parsed values are
        // never re-serialized into the output document. See PARITY-NOTES.
        assert_eq!(parse("-0").unwrap(), Value::Double(-0.0));
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

    /// serde_json's recursion limit, which replaced this module's own
    /// MAX_PARSE_DEPTH when parsing moved to serde_json.
    const SERDE_RECURSION_LIMIT: usize = 128;

    #[test]
    fn parse_depth_is_capped_instead_of_overflowing_the_stack() {
        // At the cap the document still parses...
        let deep_ok = format!(
            "{}1{}",
            "[".repeat(SERDE_RECURSION_LIMIT - 1),
            "]".repeat(SERDE_RECURSION_LIMIT - 1)
        );
        assert!(parse(&deep_ok).is_ok());

        // ...beyond it a clean Err, NOT a stack overflow. That property is why
        // a cap has to exist at all: without one this input class aborts the
        // host process, since a Windows stack overflow is an SEH exception
        // `capi`'s catch_unwind cannot contain. serde_json caps at 128 where
        // the hand-written parser capped at MAX_PARSE_DEPTH (512), so documents
        // nested between the two now fail where they used to parse.
        let too_deep = format!(
            "{}1{}",
            "[".repeat(SERDE_RECURSION_LIMIT),
            "]".repeat(SERDE_RECURSION_LIMIT)
        );
        assert!(parse(&too_deep).is_err());
        assert!(parse(&format!("{}1{}", "[".repeat(600), "]".repeat(600))).is_err());

        // The unterminated form a hostile meta.ini would actually carry.
        assert!(parse(&"[".repeat(100_000)).is_err());

        // Depth is per-path, not cumulative: many shallow siblings are fine.
        let wide = format!("[{}]", vec!["[1]"; 5000].join(","));
        assert!(parse(&wide).is_ok());
    }
}
