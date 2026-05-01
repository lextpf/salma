//! JSON value model for the inference output path, and the byte-level rules
//! its output has to satisfy.
//!
//! The acceptance bar is output byte-identical to `nlohmann::json::dump(2)`.
//! This module owns the [`Value`] model that meets that bar. It does not own
//! the serializer or the parser: [`Value::dump`] and [`parse`] convert at the
//! boundary and delegate to `serde_json`.
//!
//! ```text
//!   this module                     boundary          serde_json 1.0
//!   -----------------------------   --------------    -----------------------
//!   Value::Null Bool Int UInt       to_serde()  --->  Value::Number(i64|u64|f64)
//!         Double Str Array Object                     to_string_pretty -> bytes
//!   builders, accessors, dump()
//!                                   from_serde() <--  from_str  <- bytes
//!
//!   owned here : the Int / UInt / Double split, key order, the builders
//!   delegated  : dump() and parse()
//!   tests only : format_double(), which no production path calls
//! ```
//!
//! The [`Value::Int`] / [`Value::UInt`] / [`Value::Double`] split is why the
//! model is worth keeping on top of `serde_json::Value`. The same numeric zero
//! must print `0` when it is a count and `0.0` when it is a confidence
//! component, so integers and doubles have to stay distinct variants at
//! construction time. `serde_json::Number` keeps `i64`, `u64` and `f64`
//! distinct in the same way, so the split survives the hop across the boundary.
//! Collapsing [`Value`] into `serde_json::Value` would rewrite every call site
//! that picks a variant, which is the exact code this module protects.
//!
//! ## What `dump(2)` has to produce, byte for byte
//!
//! The layout, escaping and number tests in this file pin all five properties:
//!
//! - **Sorted object keys.** Members serialize in unsigned-byte lexicographic
//!   order, which for the ASCII keys used here is Rust `str` `Ord`.
//!   [`Value::Object`] stores a [`BTreeMap`], which iterates in exactly that
//!   order, so the order a builder inserts in never reaches the output.
//! - **Pretty layout (indent 2).** Two spaces per nesting level; `'\n'`
//!   newlines; an object member line is `<indent>"key": value`, colon then one
//!   space; an array element line is `<indent>value`; members and elements are
//!   joined with `,\n`; `{` and `[` are followed immediately by `\n`; the
//!   closing `}` or `]` sits on its own line at the parent indent. The document
//!   does not end with a newline.
//! - **Empty containers inline.** An empty array renders `[]` and an empty
//!   object `{}` on one line with no interior whitespace, even in pretty mode.
//! - **Integers and doubles are distinct types.** A [`Value::Int`] prints as a
//!   plain decimal (`0`, `2`, `802816`); a [`Value::Double`] always carries a
//!   decimal point (`0.0`, `1.0`, `0.54`). Picking the right variant per field
//!   is load-bearing: the same numeric zero is `"0"` as a count or size and
//!   `"0.0"` as a confidence component.
//! - **String escaping** matches nlohmann's default (`ensure_ascii=false`):
//!   `"` becomes `\"`, `\` becomes `\\`, the C0 shortcuts are `\b \f \n \r \t`,
//!   any other control byte below `0x20` becomes `\u00XX` with lowercase hex,
//!   and `/` is not escaped. Non-ASCII UTF-8 passes through as raw bytes.
//!
//! ## How doubles reach the output
//!
//! Doubles in the output document are formatted by `serde_json` (the `zmij`
//! backend in the pinned 1.0.151), not by Rust's `f64` `Display` and not by
//! [`format_double`]. `serde_json` and nlohmann's `dtoa` both emit the shortest
//! decimal string that round-trips to the same IEEE-754 double, and both keep a
//! decimal point on an integer-valued double, so `1.0` stays `1.0` and `0.0`
//! stays `0.0` instead of collapsing to `1` and `0`. Bare Rust `Display` does
//! collapse them, which is what [`format_double`] corrects for its own
//! test-only callers.
//!
//! A non-finite double has no JSON form. nlohmann emits `null` for NaN and for
//! both infinities, and `to_serde` reproduces that.
//!
//! Residual risk, documented but not exercised: `serde_json` and nlohmann each
//! choose their own magnitude threshold for switching from positional to
//! exponent notation, and the two are not known to agree. Every double the
//! engine writes is a confidence value in `[0, 1]`, and every count, size and
//! timing is a [`Value::Int`], so no written double reaches a magnitude where
//! the choice matters. A caller that puts an arbitrary `f64` into a
//! [`Value::Double`] leaves that guarantee behind.

use std::collections::BTreeMap;

/// An owned JSON value: the subset of JSON the inference output path builds.
///
/// [`Value::Int`] and [`Value::Double`] are separate variants on purpose.
/// Counts, sizes, `schema_version` and timings are integers; every confidence
/// field is a double; the two serialize differently (`0` against `0.0`).
/// Objects hold a [`BTreeMap`], so keys serialize sorted whatever order the
/// builder inserted them in.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer; prints without a decimal point.
    Int(i64),
    /// JSON integer above `i64::MAX`; prints without a decimal point.
    ///
    /// Only [`parse`] produces this variant; the assembly path builds
    /// [`Value::Int`] for every counter and size. It exists so a cached
    /// `meta.ini` blob carrying an integer in `(i64::MAX, u64::MAX]`
    /// round-trips through the Tier-1 emitter with its digits intact instead of
    /// degrading to a float.
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

    // --- type introspection ------------------------------------------------

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

    /// True if this is a number: [`Value::Int`], [`Value::UInt`] or
    /// [`Value::Double`].
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

    /// True for null, an empty array, or an empty object. A scalar is never
    /// empty, so `Int(0)` and `Str("")` are both non-empty.
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

    /// The integer payload, or `None` if not an integer.
    ///
    /// Stricter than a C-style cast in two ways: a [`Value::UInt`] above
    /// `i64::MAX` yields `None` rather than wrapping to a negative number, and
    /// a [`Value::Double`] yields `None` rather than truncating. Neither case
    /// is reachable today, since nothing outside the tests calls this and only
    /// [`parse`] ever builds a [`Value::UInt`].
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::UInt(n) => i64::try_from(*n).ok(),
            _ => None,
        }
    }

    /// The numeric payload as `f64`, widening a [`Value::Int`] or
    /// [`Value::UInt`], or `None` if not a number.
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

    /// Serialize to a UTF-8 string. Any non-zero `indent` selects the pretty
    /// layout; `indent == 0` selects the compact layout. Neither form ends with
    /// a newline.
    ///
    /// `indent` selects a mode, it does not set a width. The pretty writer is
    /// `serde_json::to_string_pretty`, fixed at two spaces per nesting level,
    /// so `dump(4)` still emits two-space indentation. Every call site in this
    /// crate passes 2.
    ///
    /// Byte parity is claimed for `dump(2)` only, and the layout, escaping and
    /// number tests in this file pin it. `dump(0)` is not
    /// `nlohmann::json::dump(0)`: nlohmann treats any `indent >= 0` as pretty,
    /// so its `dump(0)` still emits newlines and `": "` separators at zero
    /// indentation, while this returns the fully compact form that
    /// `nlohmann::json::dump()` with no argument produces.
    ///
    /// Never panics: [`to_serde`] only builds values `serde_json` can
    /// serialize.
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
/// The integer and double split survives the hop: `serde_json::Number` keeps
/// `i64`, `u64` and `f64` distinct, so a [`Value::Int`] still prints `0` while
/// a [`Value::Double`] still prints `0.0`.
///
/// A non-finite double has no JSON form. `Number::from_f64` returns `None` for
/// NaN and the infinities, and mapping that to `null` is what nlohmann emits.
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
        // feature), so keys stay sorted across the hop.
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
            // An integer token in i64 range becomes signed, one above it
            // unsigned, anything else a double.
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

/// Format a double the way nlohmann does, starting from Rust's `f64` `Display`.
///
/// Both nlohmann's `dtoa` and Rust's `f64` `Display` emit the shortest decimal
/// that round-trips to the same IEEE-754 double, so for identical bits the
/// digits agree. The one systematic difference is that Rust prints an
/// integer-valued double as `1` or `0` where nlohmann prints `1.0` or `0.0`, so
/// this appends `.0` when the shortest form carries no `.`, `e` or `E`. A
/// non-finite value comes back as `Display` writes it (`NaN`, `inf`, `-inf`)
/// with no `.0` appended; that is not valid JSON and does not need to be.
///
/// Not the serializer. [`Value::dump`] never calls this, because doubles in the
/// output document are formatted by `serde_json`. It stays as the independent
/// reference the diagnostics tests assert confidence rendering against, so a
/// change in the JSON layer cannot silently move the expected strings. It has
/// no non-test caller.
pub fn format_double(v: f64) -> String {
    let s = format!("{v}");
    if v.is_finite() && !s.contains(['.', 'e', 'E']) {
        format!("{s}.0")
    } else {
        s
    }
}

/// Parse a JSON document. Returns `Err(String)` on malformed input and never
/// panics. The one production caller, the Tier-1 `meta.ini` decoder in
/// `fomod_inference_service`, discards the candidate on `Err`.
///
/// Backed by `serde_json`, which implements the strict RFC 8259 grammar rather
/// than a lenient superset: leading zeros (`01`), a leading `+`, and a bare
/// `.5` or `1.` are all rejected, as are raw control bytes below `0x20` inside
/// a string. Accepting any of those would turn a Tier-1 miss into a Tier-1 hit
/// and change the whole output document. Duplicate object keys keep the last
/// occurrence; keys serialize back sorted regardless.
///
/// Three inputs behave differently from the reference nlohmann rules the rest
/// of this module reproduces. None is reachable from what the engine parses:
/// the fomod-plus cache blob and the install selections JSON carry only
/// strings, booleans and small integers, and a parsed value never re-enters the
/// output document.
///
/// | Input                      | Reference behavior                    | serde_json, in force          |
/// |----------------------------|---------------------------------------|-------------------------------|
/// | `-0`                       | `Int(0)`                              | `Double(-0.0)`                |
/// | nesting depth              | accepted to [`MAX_PARSE_DEPTH`] (512) | accepts 127, errors at 128    |
/// | `0.9999999999999999`       | exact bits                            | 1 ULP high, parses to `1.0`   |
///
/// The nesting limit is a decrementing counter that starts at 128 and errors
/// when it reaches zero, so 127 nested containers is the deepest document that
/// parses. `parse_depth_is_capped_instead_of_overflowing_the_stack` in this
/// file pins both sides of that boundary.
///
/// The third row is a `serde_json` limitation: its number parser is not
/// correctly rounded. It affects reading only. Serialization is exact, so
/// engine-computed doubles still render byte-identically; only a float read
/// back out of JSON text can differ. All three rows are recorded in
/// `PARITY-NOTES.md`.
pub fn parse(text: &str) -> Result<Value, String> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => Ok(from_serde(v)),
        Err(e) => Err(e.to_string()),
    }
}

/// Recorded nesting cap, referenced by the divergence table on [`parse`].
///
/// This is not the cap in force and nothing in the crate reads it. Parsing goes
/// through `serde_json`, whose recursion limit of 128 accepts at most 127
/// nested containers.
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
        // Forward slash is not escaped.
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

    // --- float oracle for format_double ------------------------------------
    //
    // Inline confidence values with awkward binary representations, each
    // paired with nlohmann's shortest-round-trip rendering. They match only if
    // the formatter is Rust `Display` plus the ".0" rule, since identical f64
    // bits give identical shortest digit sequences.
    //
    // This exercises format_double, not the JSON output path. The output path
    // goes through serde_json, and double_integer_values_get_decimal_point
    // above is the test that pins it.

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
        // The exact IEEE-754 sum `inference_diagnostics::composite_from`
        // computes for a fully forced plugin: 0.40 + 0.30 + 0.20 + 0.10 rounds
        // to 0.9999999999999999, not to 1.0. Pinned here so the rounding
        // cannot drift unnoticed.
        let composite = 0.40_f64 * 1.0 + 0.30 * 1.0 + 0.20 * 1.0 + 0.10 * 1.0;
        assert_eq!(format_double(composite), "0.9999999999999999");
    }

    // --- introspection helpers ---------------------------------------------

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
        // Each of these is a parse error under the reference nlohmann rules.
        // Accepting one would turn a Tier-1 miss into a Tier-1 hit and emit a
        // different document, so leniency here is a real bug, not a nicety.
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
        // serde_json reads "-0" as the float -0.0 where the nlohmann rules
        // give integer 0. Asserted rather than corrected: it is unreachable
        // from what the engine parses (the fomod-plus blob and the install
        // selections JSON carry names, booleans and small counts), and parsed
        // values never re-enter the output document. See PARITY-NOTES.md.
        assert_eq!(parse("-0").unwrap(), Value::Double(-0.0));
        assert_eq!(parse("10").unwrap(), Value::Int(10));
        assert_eq!(parse("1.5").unwrap(), Value::Double(1.5));
        assert_eq!(parse("1e-3").unwrap(), Value::Double(0.001));
        assert_eq!(parse("0.0").unwrap(), Value::Double(0.0));
    }

    #[test]
    fn parse_big_integers_use_uint_and_keep_their_digits() {
        // An integer above i64::MAX keeps its exact digits; degrading to a
        // double would print a mangled float. Reachable through the Tier-1
        // emitter, which echoes a cached `deselected` entry's raw `name` value
        // into the output.
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
        // Wider than u64 falls back to a double.
        assert!(matches!(
            parse("18446744073709551616").unwrap(),
            Value::Double(_)
        ));
    }

    #[test]
    fn parse_rejects_raw_control_bytes_and_signed_unicode_escapes() {
        // A raw control byte inside a string is a parse error; it has to be
        // escaped.
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

    /// serde_json's recursion limit: the nesting cap `parse` actually enforces.
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

        // ...beyond it a clean Err, not a stack overflow. That is why a cap
        // has to exist at all: without one this input class aborts the host
        // process, since a Windows stack overflow is an SEH exception that
        // `capi`'s catch_unwind cannot contain.
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
