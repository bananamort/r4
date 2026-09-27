use crate::nodes::{DecimalNumber, Expression, NumberExpression, StringExpression};

/// Represents an evaluated Expression result.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LuaValue {
    False,
    Function,
    /// A Lua 5.3/5.4 integer value.
    Integer(i64),
    Nil,
    Number(f64),
    String(Vec<u8>),
    Table,
    True,
    #[default]
    Unknown,
}

impl LuaValue {
    /// As defined in Lua, all values are considered true, except for false and nil. An option is
    /// returned as the LuaValue may be unknown, so it would return none.
    /// ```rust
    /// # use r4::process::LuaValue;
    ///
    /// // the values considered false
    /// assert!(!LuaValue::False.is_truthy().unwrap());
    /// assert!(!LuaValue::Nil.is_truthy().unwrap());
    ///
    /// // all the others are true
    /// assert!(LuaValue::True.is_truthy().unwrap());
    /// assert!(LuaValue::Table.is_truthy().unwrap());
    /// assert!(LuaValue::Number(0.0).is_truthy().unwrap());
    /// assert!(LuaValue::from("hello").is_truthy().unwrap());
    ///
    /// // unknown case
    /// assert!(LuaValue::Unknown.is_truthy().is_none());
    /// ```
    pub fn is_truthy(&self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Nil | Self::False => Some(false),
            _ => Some(true),
        }
    }

    /// If the value is unknown, this will also return unknown value. In the other case, if the
    /// value is considered truthy (see `is_truthy` function), it will call the given function to
    /// get the mapped value.
    pub fn map_if_truthy<F>(self, map: F) -> Self
    where
        F: Fn(Self) -> Self,
    {
        match self.is_truthy() {
            Some(true) => map(self),
            Some(false) => self,
            _ => Self::Unknown,
        }
    }

    /// Like the `map_if_truthy` method, except that instead of returning the same value when the
    /// it is falsy, it calls the default callback to obtain another value.
    pub fn map_if_truthy_else<F, G>(self, map: F, default: G) -> Self
    where
        F: Fn(Self) -> Self,
        G: Fn() -> Self,
    {
        match self.is_truthy() {
            Some(true) => map(self),
            Some(false) => default(),
            _ => Self::Unknown,
        }
    }

    /// Attempt to convert the Lua value into an expression node.
    pub fn to_expression(self) -> Option<Expression> {
        match self {
            Self::False => Some(Expression::from(false)),
            Self::True => Some(Expression::from(true)),
            Self::Nil => Some(Expression::nil()),
            Self::String(value) => Some(StringExpression::from_value(value).into()),
            Self::Integer(value) => Some(Expression::from(DecimalNumber::new_integer(value))),
            Self::Number(value) => Some(Expression::from(value)),
            _ => None,
        }
    }

    /// Attempt to convert the Lua value into a number value. This will convert strings when
    /// possible and return the same value otherwise.
    ///
    /// Numbers are always floats; `number_coercion_with_subtype` additionally
    /// applies the Lua 5.3/5.4 integer subtype.
    pub fn number_coercion(self) -> Self {
        number_coercion_with_subtype(self, false)
    }

    /// Attempt to convert the Lua value into a string value. This will convert numbers when
    /// possible and return the same value otherwise.
    ///
    /// Integral floats print without a fraction; `string_coercion_with_subtype`
    /// additionally applies the Lua 5.3/5.4 float subtype.
    pub fn string_coercion(self) -> Self {
        string_coercion_with_subtype(self, false)
    }

    /// Returns the length of a Lua value (equivalent to the `#` operator).
    pub fn length(&self) -> LuaValue {
        match self {
            Self::String(value) => LuaValue::Number(value.len() as f64),
            _ => LuaValue::Unknown,
        }
    }
}

impl From<bool> for LuaValue {
    fn from(value: bool) -> Self {
        if value {
            Self::True
        } else {
            Self::False
        }
    }
}

impl From<String> for LuaValue {
    fn from(value: String) -> Self {
        Self::String(value.into_bytes())
    }
}

impl From<&str> for LuaValue {
    fn from(value: &str) -> Self {
        Self::String(value.as_bytes().to_vec())
    }
}

impl From<Vec<u8>> for LuaValue {
    fn from(value: Vec<u8>) -> Self {
        Self::String(value)
    }
}

impl From<&[u8]> for LuaValue {
    fn from(value: &[u8]) -> Self {
        Self::String(value.to_vec())
    }
}

impl<const N: usize> From<&[u8; N]> for LuaValue {
    fn from(value: &[u8; N]) -> Self {
        Self::String(value.to_vec())
    }
}

impl From<f64> for LuaValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<i64> for LuaValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

/// Formats an `f64` the way Lua string coercion does (`%.14g`).
///
/// Lua prints numbers with up to 14 significant digits, switches to scientific
/// notation for very large or very small magnitudes, and drops trailing zeros.
/// Constant folding of `..` and interpolated strings goes through this, so it
/// must match the host runtime rather than Rust's shortest-round-trip `Display`.
fn format_lua_number(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }

    const PRECISION: usize = 14;

    // The exponent is read from Rust's `%e` form so rounding of values just
    // below a power of ten is handled by the formatter, not by `log10`.
    let scientific = format!("{value:e}");
    let exponent: i32 = scientific[scientific.find('e').expect("scientific form") + 1..]
        .parse()
        .expect("an integer exponent");

    if exponent < -4 || exponent >= PRECISION as i32 {
        let representation = format!("{:.*e}", PRECISION - 1, value);
        let position = representation.find('e').expect("scientific form");
        let mantissa = trim_trailing_zeros(&representation[..position]);
        let exponent: i32 = representation[position + 1..]
            .parse()
            .expect("an integer exponent");
        format!(
            "{mantissa}e{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.unsigned_abs()
        )
    } else {
        let decimals = (PRECISION as i32 - 1 - exponent).max(0) as usize;
        trim_trailing_zeros(&format!("{value:.decimals$}"))
    }
}

fn trim_trailing_zeros(text: &str) -> String {
    if !text.contains('.') {
        return text.to_owned();
    }
    let trimmed = text.trim_end_matches('0');
    trimmed.strip_suffix('.').unwrap_or(trimmed).to_owned()
}

/// Formats an `f64` the way Lua string coercion does, optionally keeping the
/// Lua 5.3/5.4 float subtype.
///
/// Lua 5.3/5.4 format a float with `%.14g` and append `.0` when the result
/// looks like an integer, so `tostring(1.0)` is `"1.0"` while `tostring(1)` is
/// `"1"`. Other dialects (5.1/5.2/Luau) print `"1"`, so `lua_float_subtype`
/// selects the behavior.
fn format_lua_number_with_subtype(value: f64, lua_float_subtype: bool) -> String {
    let formatted = format_lua_number(value);
    if lua_float_subtype && formatted.chars().all(|c| c == '-' || c.is_ascii_digit()) {
        format!("{formatted}.0")
    } else {
        formatted
    }
}

/// Coerces a value to a string the way `..` does, with an optional Lua
/// 5.3/5.4 float subtype for the number case.
pub(crate) fn string_coercion_with_subtype(value: LuaValue, lua_float_subtype: bool) -> LuaValue {
    let coerced = match &value {
        LuaValue::Integer(integer) => Some(LuaValue::from(integer.to_string())),
        LuaValue::Number(number) => Some(LuaValue::from(format_lua_number_with_subtype(
            *number,
            lua_float_subtype,
        ))),
        _ => None,
    };
    coerced.unwrap_or(value)
}

/// Coerces a value to a number the way arithmetic does, with an optional Lua
/// 5.3/5.4 integer subtype for integer numerals.
pub(crate) fn number_coercion_with_subtype(value: LuaValue, integer_subtype: bool) -> LuaValue {
    let parsed = match &value {
        LuaValue::String(string) => str::from_utf8(string)
            .ok()
            .map(str::trim)
            .and_then(|text| parse_number_text(text, integer_subtype)),
        _ => None,
    };
    parsed.unwrap_or(value)
}

fn parse_number_text(text: &str, integer_subtype: bool) -> Option<LuaValue> {
    let (negative, magnitude) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let number = magnitude.parse::<NumberExpression>().ok()?;
    if integer_subtype {
        if let Some(integer) = number.compute_integer() {
            return Some(LuaValue::Integer(if negative {
                integer.wrapping_neg()
            } else {
                integer
            }));
        }
    }
    let value = number.compute_value();
    Some(LuaValue::Number(if negative { -value } else { value }))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn unknown_lua_value_is_truthy_returns_none() {
        assert!(LuaValue::Unknown.is_truthy().is_none());
    }

    #[test]
    fn false_value_is_not_truthy() {
        assert!(!LuaValue::False.is_truthy().unwrap());
    }

    #[test]
    fn nil_value_is_not_truthy() {
        assert!(!LuaValue::Nil.is_truthy().unwrap());
    }

    #[test]
    fn true_value_is_truthy() {
        assert!(LuaValue::True.is_truthy().unwrap());
    }

    #[test]
    fn zero_value_is_truthy() {
        assert!(LuaValue::Number(0_f64).is_truthy().unwrap());
    }

    #[test]
    fn string_value_is_truthy() {
        assert!(LuaValue::String(b"".to_vec()).is_truthy().unwrap());
    }

    #[test]
    fn table_value_is_truthy() {
        assert!(LuaValue::Table.is_truthy().unwrap());
    }

    mod number_coercion {
        use super::*;

        macro_rules! number_coercion {
            ($($name:ident ($string:literal) => $result:expr),*) => {
                $(
                    #[test]
                    fn $name() {
                        assert_eq!(
                            LuaValue::String($string.into()).number_coercion(),
                            LuaValue::Number($result)
                        );
                    }
                )*
            };
        }

        macro_rules! no_number_coercion {
            ($($name:ident ($string:literal)),*) => {
                $(
                    #[test]
                    fn $name() {
                        assert_eq!(
                            LuaValue::String($string.into()).number_coercion(),
                            LuaValue::String($string.into())
                        );
                    }
                )*
            };
        }

        number_coercion!(
            zero("0") => 0.0,
            integer("12") => 12.0,
            integer_with_leading_zeros("00012") => 12.0,
            integer_with_ending_space("12   ") => 12.0,
            integer_with_leading_space("  123") => 123.0,
            integer_with_leading_tab("\t123") => 123.0,
            negative_integer("-3") => -3.0,
            hex_zero("0x0") => 0.0,
            hex_integer("0xA") => 10.0,
            negative_hex_integer("-0xA") => -10.0,
            float("0.5") => 0.5,
            negative_float("-0.5") => -0.5,
            float_starting_with_dot(".5") => 0.5
        );

        no_number_coercion!(
            letter_suffix("123a"),
            hex_prefix("0x"),
            space_between_minus("- 1"),
            two_seperated_digits(" 1 2")
        );
    }

    #[test]
    fn number_string_coercion_matches_lua_percent_g() {
        for (value, expected) in [
            (1.0, "1"),
            (1.5, "1.5"),
            (100.0, "100"),
            (0.1, "0.1"),
            (1.0 / 3.0, "0.33333333333333"),
            (1e21, "1e+21"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1e13, "10000000000000"),
            (-2.5, "-2.5"),
        ] {
            assert_eq!(
                LuaValue::Number(value).string_coercion(),
                LuaValue::from(expected),
                "formatting {value}"
            );
        }
    }

    #[test]
    fn string_coercion_with_the_float_subtype_appends_a_fraction() {
        for (value, expected) in [
            (1.0, "1.0"),
            (1.5, "1.5"),
            (100.0, "100.0"),
            (1e21, "1e+21"),
            (1e-5, "1e-05"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(
                string_coercion_with_subtype(LuaValue::Number(value), true),
                LuaValue::from(expected),
                "formatting {value}"
            );
        }
        // An integer keeps its integer form, and a non-subtype dialect has no
        // `.0` suffix.
        assert_eq!(
            string_coercion_with_subtype(LuaValue::Integer(1), true),
            LuaValue::from("1")
        );
        assert_eq!(
            string_coercion_with_subtype(LuaValue::Number(1.0), false),
            LuaValue::from("1")
        );
    }

    #[test]
    fn number_coercion_with_the_integer_subtype_prefers_integers() {
        for (text, expected) in [
            ("1", LuaValue::Integer(1)),
            ("-3", LuaValue::Integer(-3)),
            ("0x10", LuaValue::Integer(16)),
            ("  7  ", LuaValue::Integer(7)),
            ("1.0", LuaValue::Number(1.0)),
            ("1e2", LuaValue::Number(100.0)),
            ("-1.5", LuaValue::Number(-1.5)),
        ] {
            assert_eq!(
                number_coercion_with_subtype(LuaValue::from(text), true),
                expected,
                "coercing {text:?}"
            );
        }
        // A non-numeric string and a non-subtype dialect fall back to the
        // float-only coercion.
        assert_eq!(
            number_coercion_with_subtype(LuaValue::from("nope"), true),
            LuaValue::from("nope")
        );
        assert_eq!(
            number_coercion_with_subtype(LuaValue::from("1"), false),
            LuaValue::Number(1.0)
        );
    }
}
