//! Exact reading of JSON numbers that stand for integers.
//!
//! A schema `integer` may legally be written `1.0` or `1e0`. `serde_json`
//! keeps those as `f64`, so `Value::as_u64` refuses them, while a blind cast
//! through `f64` rounds integers above 2^53. These helpers accept exactly the
//! values that are non-negative integers and that no rounding could have
//! changed, and refuse everything else.

use serde_json::Value;

/// Largest integer such that it and every smaller one is exact in an `f64`.
const MAX_EXACT_F64_INTEGER: f64 = 9_007_199_254_740_992.0; // 2^53

/// Longest number text [`normalized_decimal`] looks at.
const MAX_DECIMAL_TEXT_BYTES: usize = 1024;
/// Largest decimal exponent magnitude [`normalized_decimal`] accepts.
const MAX_DECIMAL_EXPONENT: i64 = 4096;

/// The value of a JSON number as a `u64`, if it is a non-negative integer.
///
/// Integers that `serde_json` holds as `u64` are returned as they are (so
/// `u64::MAX` survives). A number held as `f64` is accepted only when it has
/// no fractional part and is at most 2^53, where an `f64` stands for exactly
/// one integer; a larger float could be a rounded form of another integer.
/// Fractions, negatives, non-numbers and out-of-range values are `None`.
pub fn as_u64_exact(value: &Value) -> Option<u64> {
    let number = value.as_number()?;
    if let Some(integer) = number.as_u64() {
        return Some(integer);
    }
    let float = number.as_f64()?;
    // `as_u64` was `None`, so this is a negative integer or a float form.
    if !float.is_finite() || float < 0.0 || float.fract() != 0.0 || float > MAX_EXACT_F64_INTEGER {
        return None;
    }
    Some(float as u64)
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
            ("18446744073709551615", Some(u64::MAX)),
            ("1.5", None),
            ("-1", None),
            ("-1.0", None),
            ("1e-1", None),
            // Above 2^53 a float no longer names one integer.
            ("9007199254740994.0", None),
            ("18446744073709551616", None),
            ("1e30", None),
            ("1e400", None),
        ] {
            // `1e400` is not valid for serde_json at all; both are refusals.
            let got = serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|value| as_u64_exact(&value));
            assert_eq!(got, expected, "{text}");
        }
        assert_eq!(as_u64_exact(&json!("1")), None);
        assert_eq!(as_u64_exact(&json!(null)), None);
        assert_eq!(as_u64_exact(&parse("18446744073709551615")), Some(u64::MAX));
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
