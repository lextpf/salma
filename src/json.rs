/*!
 * @brief defines the ordered JSON value model used by inference output.
 * @author Alex (https://github.com/lextpf)
 *
 * object keys use byte-lexicographic order. integer and floating variants remain distinct so
 * counts render as 0 and confidence values render as 0.0. non-finite floats render as null.
 */

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Double(f64),
    Str(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn object() -> Value {
        Value::Object(BTreeMap::new())
    }

    pub fn array() -> Value {
        Value::Array(Vec::new())
    }

    pub fn string(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn insert(&mut self, key: impl Into<String>, val: Value) -> &mut Self {
        match self {
            Value::Object(map) => {
                map.insert(key.into(), val);
            }
            other => panic!("insert on non-object {other:?}"),
        }
        self
    }

    pub fn push(&mut self, val: Value) {
        match self {
            Value::Array(items) => items.push(val),
            other => panic!("push on non-array {other:?}"),
        }
    }

    pub fn is_string(&self) -> bool {
        matches!(self, Value::Str(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::UInt(_) | Value::Double(_))
    }

    pub fn is_boolean(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Array(items) => items.is_empty(),
            Value::Object(map) => map.is_empty(),
            _ => false,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::UInt(n) => i64::try_from(*n).ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Int(n) => Some(*n as f64),
            Value::UInt(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        matches!(self, Value::Object(map) if map.contains_key(key))
    }

    pub fn array_len(&self) -> Option<usize> {
        match self {
            Value::Array(items) => Some(items.len()),
            _ => None,
        }
    }

    pub fn get_index(&self, i: usize) -> Option<&Value> {
        match self {
            Value::Array(items) => items.get(i),
            _ => None,
        }
    }

    /**
     * @fn dump(&self, usize) -> String
     * @brief select compact output for zero indentation and fixed pretty output otherwise.
     * @author Alex (https://github.com/lextpf)
     *
     * only `dump(2)` has a byte-stability contract. layout, escaping, and number tests pin it.
     */
    pub fn dump(&self, indent: usize) -> String {
        let v = to_serde(self);
        if indent == 0 {
            serde_json::to_string(&v).expect("Value cannot fail to serialize")
        } else {
            serde_json::to_string_pretty(&v).expect("Value cannot fail to serialize")
        }
    }
}

// non-finite floats serialize as null because JSON has no representation for them.
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
        // serde_json's Map is BTreeMap-backed by default (no `preserve_order` feature), so keys
        // stay sorted across the hop.
        Value::Object(map) => {
            serde_json::Value::Object(map.iter().map(|(k, v)| (k.clone(), to_serde(v))).collect())
        }
    }
}

fn from_serde(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            // an integer token in i64 range becomes signed, one above it unsigned, anything else a
            // double.
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

pub fn format_double(v: f64) -> String {
    let s = format!("{v}");
    if v.is_finite() && !s.contains(['.', 'e', 'E']) {
        format!("{s}.0")
    } else {
        s
    }
}

/**
 * @fn parse(&str) -> Result<Value, String>
 * @brief enforce strict RFC 8259 syntax without panicking.
 * @author Alex (https://github.com/lextpf)
 *
 * backed by `serde_json`, which implements the strict RFC 8259 grammar rather than a lenient
 * superset: leading zeros (`01`), a leading `+`, and a bare `.5` or `1.` are all rejected, as are
 * raw control bytes below `0x20` inside a string.
 * @return `Err(String)` on malformed input and never panics.
 */
pub fn parse(text: &str) -> Result<Value, String> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => Ok(from_serde(v)),
        Err(e) => Err(e.to_string()),
    }
}

pub const MAX_PARSE_DEPTH: usize = 512;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_keys_emit_sorted_regardless_of_insert_order() {
        let mut obj = Value::object();
        obj.insert("zebra", Value::Int(1));
        obj.insert("alpha", Value::Int(2));
        obj.insert("mike", Value::Int(3));
        assert_eq!(
            obj.dump(2),
            "{\n  \"alpha\": 2,\n  \"mike\": 3,\n  \"zebra\": 1\n}"
        );
    }

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
        let expected = "{\n  \"arr\": [\n    1,\n    2\n  ],\n  \"obj\": {\n    \"a\": 1,\n    \"b\": 2\n  }\n}";
        assert_eq!(root.dump(2), expected);
    }

    #[test]
    fn empty_array_and_object_render_inline() {
        assert_eq!(Value::array().dump(2), "[]");
        assert_eq!(Value::object().dump(2), "{}");
        let mut root = Value::object();
        root.insert("reasons", Value::array());
        root.insert("meta", Value::object());
        assert_eq!(root.dump(2), "{\n  \"meta\": {},\n  \"reasons\": []\n}");
    }

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

    #[test]
    fn string_escaping_matches_nlohmann_default() {
        assert_eq!(Value::string("a\"b").dump(2), "\"a\\\"b\"");
        assert_eq!(Value::string("a\\b").dump(2), "\"a\\\\b\"");
        assert_eq!(Value::string("a\nb").dump(2), "\"a\\nb\"");
        assert_eq!(Value::string("a\tb").dump(2), "\"a\\tb\"");
        assert_eq!(Value::string("a\rb").dump(2), "\"a\\rb\"");
        assert_eq!(Value::string("a\u{08}b").dump(2), "\"a\\bb\"");
        assert_eq!(Value::string("a\u{0C}b").dump(2), "\"a\\fb\"");
        assert_eq!(Value::string("a\u{01}b").dump(2), "\"a\\u0001b\"");
        assert_eq!(Value::string("\u{1f}").dump(2), "\"\\u001f\"");
        assert_eq!(Value::string("a/b").dump(2), "\"a/b\"");
        assert_eq!(Value::string("café").dump(2), "\"café\"");
    }

    #[test]
    fn no_trailing_newline() {
        let mut root = Value::object();
        root.insert("k", Value::Int(1));
        let s = root.dump(2);
        assert!(s.ends_with('}'));
        assert!(!s.ends_with('\n'));
    }

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
        // the exact IEEE-754 sum `inference_diagnostics::composite_from` computes for a fully
        // forced plugin: 0.40 + 0.30 + 0.20 + 0.10 rounds to 0.9999999999999999, not to 1.0. pinned
        // here so the rounding cannot drift unnoticed.
        let composite = 0.40_f64 * 1.0 + 0.30 * 1.0 + 0.20 * 1.0 + 0.10 * 1.0;
        assert_eq!(format_double(composite), "0.9999999999999999");
    }

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

    #[test]
    fn parse_scalars_and_containers() {
        assert_eq!(parse("null").unwrap(), Value::Null);
        assert_eq!(parse("  true ").unwrap(), Value::Bool(true));
        assert_eq!(parse("false").unwrap(), Value::Bool(false));
        assert_eq!(parse("42").unwrap(), Value::Int(42));
        assert_eq!(parse("-7").unwrap(), Value::Int(-7));
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
        assert_eq!(parse(r#""😀""#).unwrap(), Value::string("\u{1f600}"));
        assert_eq!(parse("\"café\"").unwrap(), Value::string("café"));
    }

    #[test]
    fn parse_duplicate_keys_keep_last() {
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
        // accepting these non-RFC forms could turn a tier-1 miss into a hit and change its output.
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

        assert_eq!(parse("0").unwrap(), Value::Int(0));
        // `-0` parses as floating negative zero. current inputs carry names, booleans, and small
        // counts, and parsed values never re-enter the output document.
        assert_eq!(parse("-0").unwrap(), Value::Double(-0.0));
        assert_eq!(parse("10").unwrap(), Value::Int(10));
        assert_eq!(parse("1.5").unwrap(), Value::Double(1.5));
        assert_eq!(parse("1e-3").unwrap(), Value::Double(0.001));
        assert_eq!(parse("0.0").unwrap(), Value::Double(0.0));
    }

    #[test]
    fn parse_big_integers_use_uint_and_keep_their_digits() {
        // an integer above i64::MAX keeps its exact digits; degrading to a double would print a
        // mangled float. reachable through the tier-1 emitter, which echoes a cached `deselected`
        // entry's raw `name` value into the output.
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
        assert!(matches!(
            parse("18446744073709551616").unwrap(),
            Value::Double(_)
        ));
    }

    #[test]
    fn parse_rejects_raw_control_bytes_and_signed_unicode_escapes() {
        // a raw control byte inside a string is a parse error; it has to be escaped.
        assert!(parse("\"a\tb\"").is_err());
        assert!(parse("\"a\nb\"").is_err());
        assert!(parse("\"a\u{1}b\"").is_err());
        assert_eq!(parse(r#""a\tb""#).unwrap(), Value::string("a\tb"));
        assert_eq!(parse(r#""a\u0001b""#).unwrap(), Value::string("a\u{1}b"));
        // `u32::from_str_radix` would accept a sign, so `\u+123` must not decode.
        assert!(parse(r#""\u+123""#).is_err());
        assert!(parse(r#""\u 123""#).is_err());
        assert!(parse(r#""\uzzzz""#).is_err());
    }

    const SERDE_RECURSION_LIMIT: usize = 128;

    #[test]
    fn parse_depth_is_capped_instead_of_overflowing_the_stack() {
        let deep_ok = format!(
            "{}1{}",
            "[".repeat(SERDE_RECURSION_LIMIT - 1),
            "]".repeat(SERDE_RECURSION_LIMIT - 1)
        );
        assert!(parse(&deep_ok).is_ok());

        // ...beyond it a clean Err, not a stack overflow. that is why a cap has to exist at all:
        // without one this input class aborts the host process, since a windows stack overflow is
        // an SEH exception that `capi`'s catch_unwind cannot contain.
        let too_deep = format!(
            "{}1{}",
            "[".repeat(SERDE_RECURSION_LIMIT),
            "]".repeat(SERDE_RECURSION_LIMIT)
        );
        assert!(parse(&too_deep).is_err());
        assert!(parse(&format!("{}1{}", "[".repeat(600), "]".repeat(600))).is_err());

        // the unterminated form a hostile meta.ini would actually carry.
        assert!(parse(&"[".repeat(100_000)).is_err());

        let wide = format!("[{}]", vec!["[1]"; 5000].join(","));
        assert!(parse(&wide).is_ok());
    }
}
