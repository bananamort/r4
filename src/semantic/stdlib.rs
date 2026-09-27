//! A curated standard-library purity model.
//!
//! [`call_is_pure`] decides whether a call invokes a known side-effect-free
//! standard-library function. It resolves the callee through the semantic model
//! by identity: the callee must be a global (or an immutable alias of one)
//! whose name is not shadowed, and the exact library path must be in the
//! curated table below. Anything not in the table is treated as effectful — the
//! model never guesses.
//!
//! [`has_side_effects`] combines this with [`Evaluator::has_side_effects_with`]
//! so optimization passes can remove or hoist standard-library calls whose
//! results are unused or loop invariant. A call is only *removable* when it is
//! also total, which [`call_is_total`] decides: a pure call such as
//! `math.floor(x)` still raises when `x` is not a number, so the totality model
//! must accept the argument values before the call may be deleted.

use crate::nodes::{Expression, FunctionCall, Identifier, Prefix};
use crate::process::{Evaluator, LuaValue};

use super::{SemanticModel, SymbolId, SymbolKind};

/// Returns whether `call` invokes a known pure standard-library function.
pub fn call_is_pure(model: &SemanticModel, call: &FunctionCall) -> bool {
    pure_path(model, call).is_some()
}

/// Resolves `call`'s callee to a pure standard-library path.
///
/// Returns the global symbol and the field path (`math.floor` becomes
/// `["floor"]`) when the callee is a global, or an immutable alias of one, that
/// is not shadowed or reassigned and denotes a curated pure function. A
/// library *table* must additionally be unmodified: no field of it, or of an
/// immutable alias of it, is written and it never escapes as a value, so the
/// field still denotes the standard-library function.
fn pure_path<'c>(
    model: &SemanticModel,
    call: &'c FunctionCall,
) -> Option<(SymbolId, Vec<&'c str>)> {
    if call.get_method().is_some() {
        // Method calls pass a receiver; the model does not describe them.
        return None;
    }

    let (root, fields) = callee_path(call.get_prefix())?;

    let symbol = model.resolve_identifier(root)?;
    let global = model.immutable_alias_root(symbol)?;
    let global_symbol = model.symbol(global);
    if global_symbol.kind != SymbolKind::Global {
        return None;
    }
    if global_symbol.write_count != 0 {
        // The program reassigns this global, so it no longer denotes the
        // standard-library function hidden behind its name.
        return None;
    }
    if !model.name_is_transparent_global(&global_symbol.name) {
        return None;
    }
    if !is_pure_path(&global_symbol.name, &fields) {
        return None;
    }
    // A library *table* is only trusted while no code can replace its fields; a
    // bare global function is already covered by the write-count check above.
    if !fields.is_empty() && !model.global_table_is_unmodified(global) {
        return None;
    }

    Some((global, fields))
}

/// Returns whether evaluating `expression` can have side effects, treating pure
/// standard-library calls as effect free.
pub fn has_side_effects(
    model: &SemanticModel,
    evaluator: &Evaluator,
    expression: &crate::nodes::Expression,
) -> bool {
    evaluator.has_side_effects_with(expression, &|call| call_is_pure(model, call))
}

/// Returns whether evaluating `expression` is observable: it has a side effect
/// or it can raise a Lua error.
///
/// An optimization that deletes or moves an evaluation must require this to be
/// `false`. A pure expression can still raise: `a + b` fails when `a` is a
/// string, and `math.floor(x)` fails when `x` is not a number. Checking side
/// effects alone is not enough.
pub fn has_observable_effect(
    model: &SemanticModel,
    evaluator: &Evaluator,
    expression: &Expression,
) -> bool {
    has_side_effects(model, evaluator, expression)
        || !expression_is_total(model, evaluator, expression)
}

/// Returns whether evaluating `expression` cannot raise, using the
/// standard-library totality model for calls.
fn expression_is_total(
    model: &SemanticModel,
    evaluator: &Evaluator,
    expression: &Expression,
) -> bool {
    evaluator.cannot_raise_with(expression, &|call| call_is_total(model, evaluator, call))
}

/// Returns whether calling `call` cannot raise a Lua error.
///
/// The callee must be a pure standard-library function (see [`call_is_pure`])
/// and every argument must be a constant that the function accepts. An argument
/// the evaluator cannot reduce to a constant is conservatively rejected, because
/// the runtime type check could fail.
///
/// Only functions whose sole failure mode is an argument *type* mismatch are
/// modelled. A function that can also raise a value error — `string.rep` with a
/// negative count, `string.char` with an out-of-range code, `utf8.len` on
/// invalid UTF-8, `string.find` with a malformed pattern — is deliberately
/// absent, so it is never reported as total. That is sound: an unmodelled call
/// is treated as able to raise.
pub fn call_is_total(model: &SemanticModel, evaluator: &Evaluator, call: &FunctionCall) -> bool {
    let Some((global, fields)) = pure_path(model, call) else {
        return false;
    };
    let Some(contract) = total_signature(&model.symbol(global).name, &fields) else {
        return false;
    };

    let arguments = call.get_arguments().clone().to_expressions();
    if !contract.accepts_count(arguments.len()) {
        return false;
    }

    arguments.iter().enumerate().all(|(index, argument)| {
        if !expression_is_total(model, evaluator, argument) {
            return false;
        }
        let kind = contract.kind_at(index);
        if kind == ArgumentKind::Any {
            return true;
        }
        match evaluator.evaluate_with(argument, &|_| None) {
            Some(value) => kind.accepts(&value),
            None => false,
        }
    })
}

/// The type a standard-library parameter accepts.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ArgumentKind {
    /// Any value is accepted without a type error.
    Any,
    /// A number.
    Number,
    /// An integer-valued number.
    Integer,
    /// A string.
    String,
    /// A table.
    Table,
    /// A table or a string.
    TableOrString,
}

impl ArgumentKind {
    /// Returns whether a constant `value` of this kind is accepted.
    ///
    /// The check is deliberately stricter than Lua's coercion rules so that it
    /// holds for every dialect r4 supports: a numeric string is not treated as a
    /// number, nor a number as a string.
    fn accepts(self, value: &LuaValue) -> bool {
        match self {
            Self::Any => true,
            Self::Number => matches!(value, LuaValue::Number(_) | LuaValue::Integer(_)),
            Self::Integer => is_integer_value(value),
            Self::String => matches!(value, LuaValue::String(_)),
            Self::Table => matches!(value, LuaValue::Table),
            Self::TableOrString => matches!(value, LuaValue::Table | LuaValue::String(_)),
        }
    }
}

/// Returns whether `value` is an integral number. Both the truncating and the
/// strict integer conversions accept such a value, so a parameter checked as an
/// integer is satisfied.
fn is_integer_value(value: &LuaValue) -> bool {
    match value {
        LuaValue::Integer(_) => true,
        LuaValue::Number(number) => number.is_finite() && number.fract() == 0.0,
        _ => false,
    }
}

/// The argument contract of a total standard-library function.
struct Signature {
    min_arguments: usize,
    /// `None` accepts any larger argument count, matching Lua's habit of
    /// ignoring extra arguments.
    max_arguments: Option<usize>,
    /// The kinds of the leading positions; the final kind repeats for extra
    /// arguments.
    kinds: &'static [ArgumentKind],
    rest_kind: ArgumentKind,
}

impl Signature {
    /// A signature with fixed positions and no extra arguments.
    const fn exact(kinds: &'static [ArgumentKind]) -> Self {
        Self {
            min_arguments: kinds.len(),
            max_arguments: Some(kinds.len()),
            kinds,
            rest_kind: ArgumentKind::Any,
        }
    }

    /// A signature that requires `min` arguments and then accepts any number of
    /// `rest_kind` arguments.
    const fn variadic(min: usize, kinds: &'static [ArgumentKind], rest_kind: ArgumentKind) -> Self {
        Self {
            min_arguments: min,
            max_arguments: None,
            kinds,
            rest_kind,
        }
    }

    fn accepts_count(&self, count: usize) -> bool {
        count >= self.min_arguments && self.max_arguments.is_none_or(|max| count <= max)
    }

    fn kind_at(&self, index: usize) -> ArgumentKind {
        self.kinds.get(index).copied().unwrap_or(self.rest_kind)
    }
}

/// The totality contract for a pure standard-library path.
///
/// Returns `None` for a function that can raise a value error, so callers treat
/// it as able to raise.
fn total_signature(root: &str, fields: &[&str]) -> Option<Signature> {
    let signature = match (root, fields) {
        // These functions accept any value, so only the argument count matters.
        ("type", []) => Signature::variadic(1, &[ArgumentKind::Any], ArgumentKind::Any),
        ("getmetatable", []) => Signature::variadic(1, &[ArgumentKind::Any], ArgumentKind::Any),
        ("rawequal", []) => Signature::variadic(
            2,
            &[ArgumentKind::Any, ArgumentKind::Any],
            ArgumentKind::Any,
        ),
        ("rawlen", []) => Signature::variadic(
            1,
            &[ArgumentKind::TableOrString],
            ArgumentKind::TableOrString,
        ),
        ("rawget", []) => Signature::variadic(
            2,
            &[ArgumentKind::Table, ArgumentKind::Any],
            ArgumentKind::Any,
        ),
        // `tonumber(v, base)` validates `base`, so only the one-argument form is
        // total.
        ("tonumber", []) => Signature::exact(&[ArgumentKind::Any]),

        // A `math` function only raises when an argument is not a number, so
        // requiring numbers makes the call total. `call_is_pure` has already
        // restricted the field to the pure `math` functions.
        ("math", [field]) => {
            let minimum = match *field {
                "atan2" | "fmod" | "ldexp" | "pow" => 2,
                "clamp" => 3,
                _ => 1,
            };
            Signature::variadic(minimum, &[ArgumentKind::Number], ArgumentKind::Number)
        }

        // Only the string functions with neither pattern nor range checks are
        // modelled.
        ("string", [field]) => match *field {
            "len" | "lower" | "upper" | "reverse" => {
                Signature::variadic(1, &[ArgumentKind::String], ArgumentKind::String)
            }
            "byte" => Signature::variadic(
                1,
                &[
                    ArgumentKind::String,
                    ArgumentKind::Integer,
                    ArgumentKind::Integer,
                ],
                ArgumentKind::Any,
            ),
            "sub" => Signature::variadic(
                2,
                &[
                    ArgumentKind::String,
                    ArgumentKind::Integer,
                    ArgumentKind::Integer,
                ],
                ArgumentKind::Any,
            ),
            _ => return None,
        },

        // `table.pack` never fails, whatever it is given.
        ("table", ["pack"]) => Signature::variadic(0, &[], ArgumentKind::Any),

        ("os", ["clock"]) => Signature::variadic(0, &[], ArgumentKind::Any),
        ("os", ["getenv"]) => Signature::variadic(1, &[ArgumentKind::String], ArgumentKind::String),
        ("os", ["difftime"]) => Signature::variadic(
            1,
            &[ArgumentKind::Number, ArgumentKind::Number],
            ArgumentKind::Number,
        ),
        // `os.time(table)` validates the table's fields, so only `os.time()` is
        // total.
        ("os", ["time"]) => Signature::exact(&[]),

        _ => return None,
    };

    Some(signature)
}

fn callee_path(prefix: &Prefix) -> Option<(&Identifier, Vec<&str>)> {
    match prefix {
        Prefix::Identifier(identifier) => Some((identifier, Vec::new())),
        Prefix::Field(field) => {
            let (root, mut fields) = callee_path(field.get_prefix())?;
            fields.push(field.get_field().get_name().as_str());
            Some((root, fields))
        }
        _ => None,
    }
}

fn is_pure_path(root: &str, fields: &[&str]) -> bool {
    match fields {
        // `tostring` is deliberately absent: it dispatches `__tostring`, so a
        // call can run arbitrary user code and must not be treated as pure.
        [] => matches!(
            root,
            "type" | "tonumber" | "rawequal" | "rawlen" | "rawget" | "getmetatable" | "select"
        ),
        [field] => match root {
            "math" => matches!(
                *field,
                "abs"
                    | "acos"
                    | "asin"
                    | "atan"
                    | "atan2"
                    | "ceil"
                    | "clamp"
                    | "cos"
                    | "cosh"
                    | "deg"
                    | "exp"
                    | "floor"
                    | "fmod"
                    | "frexp"
                    | "ldexp"
                    | "log"
                    | "log10"
                    | "max"
                    | "min"
                    | "modf"
                    | "noise"
                    | "pow"
                    | "rad"
                    | "round"
                    | "sign"
                    | "sin"
                    | "sinh"
                    | "sqrt"
                    | "tan"
                    | "tanh"
            ),
            "string" => matches!(
                *field,
                // `format` is deliberately absent: `%s` converts through
                // `tostring`, which can dispatch `__tostring`.
                "byte"
                    | "char"
                    | "find"
                    | "len"
                    | "lower"
                    | "match"
                    | "rep"
                    | "reverse"
                    | "split"
                    | "sub"
                    | "upper"
                    | "pack"
                    | "unpack"
                    | "packsize"
            ),
            // `concat` is deliberately absent: it converts elements through
            // `tostring`, which can dispatch `__tostring`.
            "table" => matches!(*field, "pack" | "unpack"),
            "utf8" => matches!(
                *field,
                "char"
                    | "codepoint"
                    | "len"
                    | "offset"
                    | "codes"
                    | "nfcnormalize"
                    | "nfdnormalize"
                    | "graphemes"
            ),
            "bit32" => matches!(
                *field,
                "arshift"
                    | "band"
                    | "bnot"
                    | "bor"
                    | "btest"
                    | "bxor"
                    | "extract"
                    | "lrotate"
                    | "lshift"
                    | "replace"
                    | "rrotate"
                    | "rshift"
            ),
            "os" => matches!(*field, "clock" | "date" | "difftime" | "getenv" | "time"),
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::nodes::LastStatement;
    use crate::Parser;

    fn returned_expression(code: &str) -> (SemanticModel, Expression) {
        let (block, model) = Parser::default()
            .parse_analyzed(code)
            .unwrap_or_else(|error| panic!("`{code}` must analyze: {error}"));
        let Some(LastStatement::Return(statement)) = block.get_last_statement() else {
            panic!("`{code}` must end with a return");
        };
        let expression = statement
            .iter_expressions()
            .next()
            .cloned()
            .unwrap_or_else(|| panic!("`{code}` must return a value"));
        (model, expression)
    }

    fn is_total(code: &str) -> bool {
        let (model, expression) = returned_expression(code);
        let Expression::Call(call) = &expression else {
            panic!("`{code}` must return a call");
        };
        call_is_total(&model, &Evaluator::default(), call)
    }

    fn is_observable(code: &str) -> bool {
        let (model, expression) = returned_expression(code);
        has_observable_effect(&model, &Evaluator::default(), &expression)
    }

    #[test]
    fn total_calls_are_recognised() {
        for code in [
            "return math.floor(1.5)",
            "return math.abs(-3)",
            "return math.log(1)",
            "return math.max(1, 2, 3)",
            "return math.clamp(5, 1, 3)",
            "return math.atan2(1, 2)",
            "return math.pow(2, 3)",
            "return type(value)",
            "return rawequal(1, 2)",
            "return rawlen('abc')",
            "return rawlen({})",
            "return rawget({}, 1)",
            "return getmetatable(nil)",
            "return tonumber('1')",
            "return string.len('abc')",
            "return string.lower('ABC')",
            "return string.upper('abc')",
            "return string.reverse('abc')",
            "return string.sub('abc', 1)",
            "return string.byte('abc')",
            "return string.byte('abc', 1, 2)",
            "return table.pack()",
            "return os.clock()",
            "return os.getenv('HOME')",
            "return os.difftime(1, 2)",
            "return os.time()",
        ] {
            assert!(is_total(code), "`{code}` must be total");
        }
    }

    #[test]
    fn calls_that_are_not_total_are_rejected() {
        for code in [
            // A variable has an unknown type, so a runtime type check could fail.
            "return math.floor(value)",
            // A non-numeric string is not a number, and a numeric string is not
            // treated as one either.
            "return math.floor('abc')",
            "return math.floor('1.5')",
            // A table is not a number.
            "return math.floor({})",
            // A required argument is missing.
            "return math.floor()",
            "return math.atan2(1)",
            // Not a pure function.
            "return math.random(1)",
            "return print(1)",
            // Functions with a value error are deliberately unmodelled.
            "return string.rep('a', -1)",
            "return string.char(300)",
            "return string.find('abc', '[')",
            "return utf8.len('abc')",
            "return select(1, 2)",
            "return table.unpack({ 1, 2 })",
            "return os.date('x')",
            "return os.time({})",
            // The two-argument form validates the base.
            "return tonumber('10', 16)",
            // A non-integral index fails the integer check.
            "return string.sub('abc', 1.5)",
            "return string.sub('abc', {})",
            // A non-string argument.
            "return string.len({})",
            "return string.len(5)",
            "return rawlen(5)",
        ] {
            assert!(!is_total(code), "`{code}` must not be total");
        }
    }

    #[test]
    fn a_mutated_library_table_is_not_total() {
        for code in [
            "math.floor = function() end\nreturn math.floor(1.5)",
            "math['floor'] = function() end\nreturn math.floor(1.5)",
            "math[key] = function() end\nreturn math.floor(1.5)",
            "(math).floor = function() end\nreturn math.floor(1.5)",
            "local m = math\nm.floor = function() end\nreturn math.floor(1.5)",
            "install(math)\nreturn math.floor(1.5)",
            "_G.math.floor = function() end\nreturn math.floor(1.5)",
        ] {
            assert!(!is_total(code), "`{code}` must not be total");
        }
        // An unmodified table, including through a read-only alias, is still
        // recognized.
        assert!(is_total("return math.floor(1.5)"));
        assert!(is_total("local m = math\nreturn m.floor(1.5)"));
    }

    #[test]
    fn integer_values_are_recognised() {
        assert!(is_integer_value(&LuaValue::Integer(1)));
        assert!(is_integer_value(&LuaValue::Number(2.0)));
        assert!(!is_integer_value(&LuaValue::Number(1.5)));
        assert!(!is_integer_value(&LuaValue::Number(f64::NAN)));
        assert!(!is_integer_value(&LuaValue::Number(f64::INFINITY)));
        assert!(!is_integer_value(&LuaValue::String(b"1".to_vec())));
    }

    #[test]
    fn observable_effects_distinguish_raising_expressions() {
        for (code, observable) in [
            ("return math.floor(1.5)", false),
            ("return math.floor(value)", true),
            ("return math.floor('abc')", true),
            ("return 1 + 2", false),
            ("return value + 1", true),
            ("return {}", false),
            ("return { value + 1 }", true),
            ("return print(1)", true),
            ("return nil", false),
            ("return function() end", false),
        ] {
            assert_eq!(is_observable(code), observable, "`{code}`");
        }
    }
}
