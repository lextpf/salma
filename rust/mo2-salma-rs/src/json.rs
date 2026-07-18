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
    /// JSON integer (C++ `int` / `int64_t` / `uint64_t`); prints without a
    /// decimal point.
    Int(i64),
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

    /// True if this is a number ([`Value::Int`] or [`Value::Double`]), matching
    /// nlohmann `is_number`.
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Double(_))
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

    /// The integer payload, or `None` if not an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// The numeric payload as `f64` (an [`Value::Int`] is widened), or `None` if
    /// not a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Int(n) => Some(*n as f64),
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
}
