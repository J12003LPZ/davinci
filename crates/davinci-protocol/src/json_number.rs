//! Exact reading of JSON numbers that stand for integers.
//!
//! A schema `integer` may legally be written `1.0` or `1e0`. `serde_json`
//! rounds every such token to an `f64` while parsing, and after that nothing can
//! tell `1.0` from `1.0000000000000001` or `9007199254740993.0` from
//! `9007199254740992.0`. So the decision is made on the text, before parsing:
//! [`normalize_integral_tokens`] rewrites each number token whose exact decimal
//! value is a non-negative integer that fits a `u64` to a plain integer
//! literal, which `serde_json` keeps as a `u64`. [`as_u64_exact`] then reads
//! only `u64`s. Every `f64` still in a value is therefore a fraction, a
//! negative or something out of range, and is refused.

use serde_json::Value;
use std::borrow::Cow;

/// Longest number text [`normalized_decimal`] looks at.
const MAX_DECIMAL_TEXT_BYTES: usize = 1024;
/// Largest decimal exponent magnitude [`normalized_decimal`] accepts.
const MAX_DECIMAL_EXPONENT: i64 = 4096;

/// The value of a JSON number as a `u64`, if it is a non-negative integer.
///
/// Only a number `serde_json` holds as a `u64` is accepted (so `u64::MAX`
/// survives). A number held as `f64` is refused whatever its value: it was
/// rounded from text that [`normalize_integral_tokens`] did not find to be an
/// exact integer, and a rounded float must never pass for the integer it lands
/// on. Text that reaches a value without that normalization (`json!(1.0)`, a
/// plain `serde_json::from_str("1.0")`) is therefore refused too; parse model
/// and wire text with [`parse_exact`].
pub fn as_u64_exact(value: &Value) -> Option<u64> {
    value.as_number()?.as_u64()
}

/// Parse JSON text so that every number written as an exact non-negative
/// integer (`1.0`, `1e3`, `18446744073709551615.0`) is held as that integer.
pub fn parse_exact(text: &str) -> serde_json::Result<Value> {
    serde_json::from_str(&normalize_integral_tokens(text))
}

/// Rewrite each number token of JSON `text` whose exact decimal value is a
/// non-negative integer within `u64` to a plain integer literal, without any
/// float arithmetic. Strings are skipped; every other token (fractions,
/// negatives, out of range, malformed) is left as written for the JSON parser
/// to judge. A token grows to at most 20 digits.
pub fn normalize_integral_tokens(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() {
                    match bytes[at] {
                        b'\\' => at += 2,
                        b'"' => break,
                        _ => at += 1,
                    }
                }
                at += 1;
            }
            b'-' | b'0'..=b'9' => {
                let start = at;
                while at < bytes.len()
                    && matches!(bytes[at], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    at += 1;
                }
                let token = &text[start..at];
                if token.contains(['.', 'e', 'E']) {
                    if let Some(integer) = normalized_decimal(token).and_then(|n| n.to_u64()) {
                        let out = out.get_or_insert_with(|| String::with_capacity(text.len()));
                        out.push_str(&text[copied..start]);
                        out.push_str(&integer.to_string());
                        copied = at;
                    }
                }
            }
            _ => at += 1,
        }
    }
    match out {
        Some(mut out) => {
            out.push_str(&text[copied.min(text.len())..]);
            Cow::Owned(out)
        }
        None => Cow::Borrowed(text),
    }
}

/// A decimal number in canonical form: `digits` has no leading or trailing
/// zeros (empty means zero) and the value is `digits x 10^exponent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDecimal {
    pub negative: bool,
    pub digits: String,
    pub exponent: i64,
}

impl NormalizedDecimal {
    /// The value as a `u64`, when it is a non-negative integer that fits.
    pub fn to_u64(&self) -> Option<u64> {
        if self.negative && !self.digits.is_empty() {
            return None;
        }
        if self.digits.is_empty() {
            return Some(0);
        }
        if self.exponent < 0 {
            return None;
        }
        let zeros = usize::try_from(self.exponent).ok()?;
        // u64::MAX has 20 digits; anything longer cannot fit.
        if self.digits.len().checked_add(zeros)? > 20 {
            return None;
        }
        let mut text = self.digits.clone();
        text.extend(std::iter::repeat_n('0', zeros));
        text.parse().ok()
    }
}

/// Parse JSON number text (`-12.50e+3`) into canonical form without going
/// through `f64`. The text length and the exponent are bounded; text outside
/// the JSON number grammar, or past either bound, is `None`.
pub fn normalized_decimal(text: &str) -> Option<NormalizedDecimal> {
    if text.is_empty() || text.len() > MAX_DECIMAL_TEXT_BYTES {
        return None;
    }
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent_text) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], Some(&rest[at + 1..])),
        None => (rest, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (mantissa, None),
    };
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(whole) || (whole.len() > 1 && whole.starts_with('0')) {
        return None;
    }
    if fraction.is_some_and(|fraction| !all_digits(fraction)) {
        return None;
    }
    let mut exponent: i64 = match exponent_text {
        None => 0,
        Some(raw) => {
            let (sign, digits) = match raw.as_bytes().first()? {
                b'-' => (-1, &raw[1..]),
                b'+' => (1, &raw[1..]),
                _ => (1, raw),
            };
            if !all_digits(digits) || digits.len() > 6 {
                return None;
            }
            sign * digits.parse::<i64>().ok()?
        }
    };
    let fraction = fraction.unwrap_or("");
    exponent = exponent.checked_sub(i64::try_from(fraction.len()).ok()?)?;
    let combined = format!("{whole}{fraction}");
    let trimmed = combined.trim_start_matches('0');
    let without_zeros = trimmed.trim_end_matches('0');
    let removed = i64::try_from(trimmed.len() - without_zeros.len()).ok()?;
    if without_zeros.is_empty() {
        return Some(NormalizedDecimal {
            negative,
            digits: String::new(),
            exponent: 0,
        });
    }
    exponent = exponent.checked_add(removed)?;
    if exponent.abs() > MAX_DECIMAL_EXPONENT {
        return None;
    }
    Some(NormalizedDecimal {
        negative,
        digits: without_zeros.to_string(),
        exponent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn audit_wp02_b023_integral_decimals_are_exact_or_refused() {
        for (text, expected) in [
            ("1", Some(1)),
            ("1.0", Some(1)),
            ("1e0", Some(1)),
            ("2.5e1", Some(25)),
            ("0", Some(0)),
            ("-0.0", Some(0)),
            ("9007199254740992.0", Some(9_007_199_254_740_992)),
            // Integral decimals past 2^53 are exact in text and stay exact.
            ("9007199254740993.0", Some(9_007_199_254_740_993)),
            ("9007199254740994.0", Some(9_007_199_254_740_994)),
            ("18446744073709551615", Some(u64::MAX)),
            ("18446744073709551615.0", Some(u64::MAX)),
            ("1844674407370955161.5e1", Some(u64::MAX)),
            ("1e19", Some(10_000_000_000_000_000_000)),
            // A fraction that rounds to an integer in an f64 is still a fraction.
            ("1.0000000000000001", None),
            ("0.9999999999999999999", None),
            ("9007199254740993.5", None),
            ("1.5", None),
            ("-1", None),
            ("-1.0", None),
            ("1e-1", None),
            ("18446744073709551616", None),
            ("18446744073709551616.0", None),
            ("1e20", None),
            ("1e30", None),
            ("1e400", None),
        ] {
            let got = parse_exact(text)
                .ok()
                .and_then(|value| as_u64_exact(&value));
            assert_eq!(got, expected, "{text}");
        }
        // A value that never saw the normalization keeps no float.
        assert_eq!(as_u64_exact(&parse("1.0")), None);
        assert_eq!(as_u64_exact(&json!(1.0)), None);
        assert_eq!(as_u64_exact(&json!("1")), None);
        assert_eq!(as_u64_exact(&json!(null)), None);
        assert_eq!(as_u64_exact(&parse("18446744073709551615")), Some(u64::MAX));
    }

    #[test]
    fn audit_wp02_b023_normalization_rewrites_only_number_tokens() {
        // Strings, escapes and keys that look like numbers are not touched.
        let text = r#"{"a":"1.0","b\"1.0":2.0,"c":[1e2,"x\\",3.0],"d":1.5,"e":-2.0}"#;
        let value = parse_exact(text).unwrap();
        assert_eq!(value["a"], "1.0");
        assert_eq!(as_u64_exact(&value["b\"1.0"]), Some(2));
        assert_eq!(as_u64_exact(&value["c"][0]), Some(100));
        assert_eq!(value["c"][1], "x\\");
        assert_eq!(as_u64_exact(&value["c"][2]), Some(3));
        assert_eq!(value["d"].as_f64(), Some(1.5));
        assert_eq!(as_u64_exact(&value["d"]), None);
        assert_eq!(as_u64_exact(&value["e"]), None);
        // Text with nothing to rewrite is returned as it came.
        assert!(matches!(
            normalize_integral_tokens(r#"{"a":1,"b":"x"}"#),
            Cow::Borrowed(_)
        ));
        assert_eq!(normalize_integral_tokens("[1.0,2e0]"), "[1,2]");
        // Malformed text is left for the parser to refuse.
        for bad in ["{\"a\":1.0", "[1..0]", "[1e]", "{\"a\":\"unterminated"] {
            assert!(parse_exact(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn audit_wp02_b023_normalized_decimal_is_checked_and_bounded() {
        let norm = |text: &str| normalized_decimal(text);
        let value = |text: &str| norm(text).and_then(|n| n.to_u64());
        assert_eq!(value("10e-1"), Some(1));
        assert_eq!(value("0.5e1"), Some(5));
        assert_eq!(value("1200e-2"), Some(12));
        assert_eq!(value("0e99999"), Some(0));
        assert_eq!(value("18446744073709551615"), Some(u64::MAX));
        assert_eq!(value("18446744073709551615.0"), Some(u64::MAX));
        assert_eq!(value("18446744073709551616"), None);
        assert_eq!(value("1.5"), None);
        assert_eq!(value("-3"), None);
        assert_eq!(value("1e19"), Some(10_000_000_000_000_000_000));
        assert_eq!(value("1e20"), None);
        assert_eq!(
            norm("-12.50e+3").unwrap(),
            NormalizedDecimal {
                negative: true,
                digits: "125".into(),
                exponent: 2
            }
        );
        // Exponent and length bounds, and text outside the JSON grammar.
        assert_eq!(norm("1e999999999999"), None);
        assert_eq!(norm("1e5000"), None);
        assert_eq!(norm(&format!("1{}", "0".repeat(2000))), None);
        for bad in [
            "", "-", "+1", "01", "1.", ".5", "1e", "1e+", "1_0", "0x10", "1e1.5", "NaN",
        ] {
            assert_eq!(norm(bad), None, "{bad}");
        }
    }
}
