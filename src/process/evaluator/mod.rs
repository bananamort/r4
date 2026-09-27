mod lua_value;

pub use lua_value::*;

use std::cmp::Ordering;

use crate::nodes::*;

/// A struct to convert an Expression node into a LuaValue object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evaluator {
    pure_metamethods: bool,
    /// When set, number literals are Lua 5.3/5.4 integers and arithmetic follows
    /// the integer rules instead of always producing floats.
    integers: bool,
}

impl Evaluator {
    /// Enables Lua 5.3/5.4 integer number semantics.
    pub fn with_integer_numbers(mut self) -> Self {
        self.integers = true;
        self
    }

    /// Returns whether Lua 5.3/5.4 integer number semantics are enabled.
    #[inline]
    pub fn integer_numbers(&self) -> bool {
        self.integers
    }

    /// Converts a folded value into an expression.
    ///
    /// Under Lua 5.3/5.4 integer semantics an integral result of a float
    /// operation is kept as a float literal (`1024.0`), so folding preserves the
    /// number subtype instead of producing an integer-looking literal that
    /// would change it.
    pub fn value_to_expression(&self, value: LuaValue) -> Option<Expression> {
        match value {
            LuaValue::Number(number)
                if self.integers && number.is_finite() && number.fract() == 0.0 =>
            {
                Some(DecimalNumber::new(number).with_fraction().into())
            }
            value => value.to_expression(),
        }
    }

    /// When evaluating expressions related to tables, this value tells the evaluator if
    /// metamethods can have side effects. For example, indexing a normal table in Lua does not
    /// have any side effects, but if the table is a metatable, it's `__index` metamethod can
    /// possibly have side effects (since it can be a function call).
    pub fn assume_pure_metamethods(mut self) -> Self {
        self.pure_metamethods = true;
        self
    }

    /// Evaluates a number literal according to the enabled number semantics.
    fn number_value(&self, number: &NumberExpression) -> LuaValue {
        if self.integers {
            if let Some(integer) = number.compute_integer() {
                return LuaValue::Integer(integer);
            }
        }
        LuaValue::Number(number.compute_value())
    }

    pub fn evaluate(&self, expression: &Expression) -> LuaValue {
        match expression {
            Expression::False(_) => LuaValue::False,
            Expression::Function(_) => LuaValue::Function,
            Expression::Nil(_) => LuaValue::Nil,
            Expression::Number(number) => self.number_value(number),
            Expression::String(string) => LuaValue::from(string.get_value()),
            Expression::Table(_) => LuaValue::Table,
            Expression::True(_) => LuaValue::True,
            Expression::Binary(binary) => self.evaluate_binary(binary),
            Expression::Unary(unary) => self.evaluate_unary(unary),
            Expression::Parenthese(parenthese) => self.evaluate_parenthese_expression(parenthese),
            Expression::If(if_expression) => self.evaluate_if(if_expression),
            Expression::InterpolatedString(interpolated_string) => {
                let mut result = Vec::new();
                for segment in interpolated_string.iter_segments() {
                    match segment {
                        InterpolationSegment::String(string) => {
                            result.extend_from_slice(string.get_value());
                        }
                        InterpolationSegment::Value(value) => {
                            match self.evaluate(value.get_expression()) {
                                LuaValue::False => {
                                    result.extend_from_slice(b"false");
                                }
                                LuaValue::True => {
                                    result.extend_from_slice(b"true");
                                }
                                LuaValue::Nil => {
                                    result.extend_from_slice(b"nil");
                                }
                                LuaValue::String(string) => {
                                    result.extend_from_slice(&string);
                                }
                                LuaValue::Function
                                | LuaValue::Integer(_)
                                | LuaValue::Number(_)
                                | LuaValue::Table
                                | LuaValue::Unknown => return LuaValue::Unknown,
                            }
                        }
                    }
                }
                LuaValue::String(result)
            }
            Expression::TypeCast(type_cast) => self.evaluate(type_cast.get_expression()),
            Expression::TypeInstantiation(type_instantiation) => {
                self.evaluate_type_instantiation(type_instantiation)
            }
            Expression::Call(_)
            | Expression::Field(_)
            | Expression::Identifier(_)
            | Expression::Index(_)
            | Expression::VariableArguments(_) => LuaValue::Unknown,
        }
    }

    fn evaluate_type_instantiation(
        &self,
        type_instantiation: &TypeInstantiationExpression,
    ) -> LuaValue {
        // when the evaluator will be able to manage tuples, keep only the first element
        // of the tuple here (or coerce the tuple to `nil` if it is empty)
        self.evaluate_prefix(type_instantiation.get_prefix())
    }

    fn evaluate_parenthese_expression(&self, parenthese: &ParentheseExpression) -> LuaValue {
        // when the evaluator will be able to manage tuples, keep only the first element
        // of the tuple here (or coerce the tuple to `nil` if it is empty)
        self.evaluate(parenthese.inner_expression())
    }

    fn evaluate_prefix(&self, prefix: &Prefix) -> LuaValue {
        match prefix {
            Prefix::Call(_) | Prefix::Field(_) | Prefix::Identifier(_) | Prefix::Index(_) => {
                LuaValue::Unknown
            }
            Prefix::Parenthese(parenthese) => self.evaluate_parenthese_expression(parenthese),
            Prefix::TypeInstantiation(type_instantiation) => {
                self.evaluate_type_instantiation(type_instantiation)
            }
        }
    }

    pub fn can_return_multiple_values(&self, expression: &Expression) -> bool {
        // In Lua, only calls and `...` can produce more than one value. Every
        // other expression form, including unary and binary operators (which
        // adjust their operands to a single value), yields exactly one value.
        match expression {
            Expression::Call(_) | Expression::VariableArguments(_) => true,
            Expression::Binary(_)
            | Expression::False(_)
            | Expression::Field(_)
            | Expression::Function(_)
            | Expression::Identifier(_)
            | Expression::If(_)
            | Expression::Index(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::Parenthese(_)
            | Expression::String(_)
            | Expression::InterpolatedString(_)
            | Expression::Table(_)
            | Expression::True(_)
            | Expression::TypeCast(_)
            | Expression::TypeInstantiation(_)
            | Expression::Unary(_) => false,
        }
    }

    /// Returns whether evaluating `expression` is mathematically pure.
    ///
    /// A pure expression cannot call functions, read mutable tables (fields or
    /// indexes), allocate a fresh table or closure, observe variable arguments,
    /// or have any other side effect. This is the strictest purity check the
    /// evaluator exposes; anything it does not recognize is impure.
    pub fn is_pure(&self, expression: &Expression) -> bool {
        match expression {
            Expression::False(_)
            | Expression::Identifier(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::String(_)
            | Expression::True(_) => true,
            Expression::Binary(binary) => {
                self.is_pure(binary.left()) && self.is_pure(binary.right())
            }
            Expression::If(if_expression) => {
                self.is_pure(if_expression.get_condition())
                    && self.is_pure(if_expression.get_result())
                    && if_expression.iter_branches().all(|branch| {
                        self.is_pure(branch.get_condition()) && self.is_pure(branch.get_result())
                    })
                    && self.is_pure(if_expression.get_else_result())
            }
            Expression::Parenthese(parenthese) => self.is_pure(parenthese.inner_expression()),
            Expression::TypeCast(type_cast) => self.is_pure(type_cast.get_expression()),
            Expression::TypeInstantiation(type_instantiation) => {
                self.is_pure_prefix(type_instantiation.get_prefix())
            }
            Expression::Unary(unary) => self.is_pure(unary.get_expression()),
            // Field/index reads observe mutable tables, calls and closures may
            // have arbitrary effects, tables and interpolated strings allocate
            // or call, and `...` depends on the caller.
            Expression::Call(_)
            | Expression::Field(_)
            | Expression::Function(_)
            | Expression::Index(_)
            | Expression::InterpolatedString(_)
            | Expression::Table(_)
            | Expression::VariableArguments(_) => false,
        }
    }

    fn is_pure_prefix(&self, prefix: &Prefix) -> bool {
        match prefix {
            Prefix::Identifier(_) => true,
            Prefix::Parenthese(parenthese) => self.is_pure(parenthese.inner_expression()),
            Prefix::TypeInstantiation(type_instantiation) => {
                self.is_pure_prefix(type_instantiation.get_prefix())
            }
            // Calls evaluate arbitrary code; field and index reads observe
            // mutable tables.
            Prefix::Call(_) | Prefix::Field(_) | Prefix::Index(_) => false,
        }
    }

    pub fn has_side_effects(&self, expression: &Expression) -> bool {
        self.has_side_effects_with(expression, &|_| false)
    }

    /// Returns whether evaluating `expression` has side effects, treating calls
    /// accepted by `pure_call` as effect free.
    ///
    /// `pure_call` must only accept calls whose callee is a known pure
    /// function; the arguments are still checked here, so a pure function
    /// called with an effectful argument is correctly reported as effectful.
    pub fn has_side_effects_with(
        &self,
        expression: &Expression,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        match expression {
            Expression::False(_)
            | Expression::Function(_)
            | Expression::Identifier(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::String(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => false,
            Expression::If(if_expression) => {
                self.if_expression_has_side_effects(if_expression, pure_call)
            }
            Expression::Binary(binary) => {
                let left = binary.left();
                let right = binary.right();

                let left_value = self.evaluate(left);
                let left_side_effect = self.has_side_effects_with(left, pure_call);

                match binary.operator() {
                    BinaryOperator::And => {
                        if left_value.is_truthy().unwrap_or(true) {
                            left_side_effect || self.has_side_effects_with(right, pure_call)
                        } else {
                            left_side_effect
                        }
                    }
                    BinaryOperator::Or => {
                        if left_value.is_truthy().unwrap_or(false) {
                            left_side_effect
                        } else {
                            left_side_effect || self.has_side_effects_with(right, pure_call)
                        }
                    }
                    _ => {
                        if self.pure_metamethods {
                            left_side_effect || self.has_side_effects_with(right, pure_call)
                        } else {
                            self.maybe_metatable(&left_value)
                                || self.maybe_metatable(&self.evaluate(right))
                                || self.has_side_effects_with(left, pure_call)
                                || self.has_side_effects_with(right, pure_call)
                        }
                    }
                }
            }
            Expression::Unary(unary) => {
                if self.pure_metamethods || matches!(unary.operator(), UnaryOperator::Not) {
                    self.has_side_effects_with(unary.get_expression(), pure_call)
                } else {
                    let sub_expression = unary.get_expression();

                    self.maybe_metatable(&self.evaluate(sub_expression))
                        || self.has_side_effects_with(sub_expression, pure_call)
                }
            }
            Expression::Field(field) => self.field_has_side_effects(field, pure_call),
            Expression::Index(index) => self.index_has_side_effects(index, pure_call),
            Expression::Parenthese(parenthese) => {
                self.has_side_effects_with(parenthese.inner_expression(), pure_call)
            }
            Expression::Table(table) => table
                .get_entries()
                .iter()
                .any(|entry| self.table_entry_has_side_effects(entry, pure_call)),
            Expression::Call(call) => self.call_has_side_effects(call, pure_call),
            Expression::InterpolatedString(interpolated_string) => interpolated_string
                .iter_segments()
                .any(|segment| match segment {
                    InterpolationSegment::String(_) => false,
                    InterpolationSegment::Value(value) => {
                        self.has_side_effects_with(value.get_expression(), pure_call)
                    }
                }),
            Expression::TypeCast(type_cast) => {
                self.has_side_effects_with(type_cast.get_expression(), pure_call)
            }
            Expression::TypeInstantiation(type_instantiation) => {
                self.type_instantiation_has_side_effects(type_instantiation, pure_call)
            }
        }
    }

    fn if_expression_has_side_effects(
        &self,
        if_expression: &IfExpression,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        if self.has_side_effects_with(if_expression.get_condition(), pure_call) {
            return true;
        }

        let condition = self.evaluate(if_expression.get_condition());

        if let Some(truthy) = condition.is_truthy() {
            if truthy {
                self.has_side_effects_with(if_expression.get_result(), pure_call)
            } else {
                for branch in if_expression.iter_branches() {
                    if self.has_side_effects_with(branch.get_condition(), pure_call) {
                        return true;
                    }

                    let branch_condition = self.evaluate(branch.get_condition());

                    if let Some(truthy) = branch_condition.is_truthy() {
                        if truthy {
                            return self.has_side_effects_with(branch.get_result(), pure_call);
                        }
                    } else if self.has_side_effects_with(branch.get_result(), pure_call) {
                        return true;
                    }
                }

                self.has_side_effects_with(if_expression.get_else_result(), pure_call)
            }
        } else {
            if self.has_side_effects_with(if_expression.get_result(), pure_call) {
                return true;
            }

            for branch in if_expression.iter_branches() {
                if self.has_side_effects_with(branch.get_condition(), pure_call)
                    || self.has_side_effects_with(branch.get_result(), pure_call)
                {
                    return true;
                }
            }

            self.has_side_effects_with(if_expression.get_else_result(), pure_call)
        }
    }

    #[inline]
    fn call_has_side_effects(
        &self,
        call: &FunctionCall,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        if !pure_call(call) {
            return true;
        }
        self.arguments_have_side_effects(call.get_arguments(), pure_call)
    }

    fn arguments_have_side_effects(
        &self,
        arguments: &Arguments,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        match arguments {
            Arguments::Tuple(tuple) => tuple
                .iter_values()
                .any(|value| self.has_side_effects_with(value, pure_call)),
            Arguments::String(_) => false,
            Arguments::Table(table) => table
                .get_entries()
                .iter()
                .any(|entry| self.table_entry_has_side_effects(entry, pure_call)),
        }
    }

    #[inline]
    fn table_entry_has_side_effects(
        &self,
        entry: &TableEntry,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        match entry {
            TableEntry::Field(entry) => self.has_side_effects_with(entry.get_value(), pure_call),
            TableEntry::Index(entry) => {
                self.has_side_effects_with(entry.get_key(), pure_call)
                    || self.has_side_effects_with(entry.get_value(), pure_call)
            }
            TableEntry::Value(value) => self.has_side_effects_with(value, pure_call),
        }
    }

    #[inline]
    fn field_has_side_effects(
        &self,
        field: &FieldExpression,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        !self.pure_metamethods || self.prefix_has_side_effects(field.get_prefix(), pure_call)
    }

    #[inline]
    fn index_has_side_effects(
        &self,
        index: &IndexExpression,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        !self.pure_metamethods
            || self.has_side_effects_with(index.get_index(), pure_call)
            || self.prefix_has_side_effects(index.get_prefix(), pure_call)
    }

    fn prefix_has_side_effects(
        &self,
        prefix: &Prefix,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        match prefix {
            Prefix::Call(call) => self.call_has_side_effects(call, pure_call),
            Prefix::Field(field) => self.field_has_side_effects(field, pure_call),
            Prefix::Identifier(_) => false,
            Prefix::Index(index) => self.index_has_side_effects(index, pure_call),
            Prefix::Parenthese(sub_expression) => {
                self.has_side_effects_with(sub_expression.inner_expression(), pure_call)
            }
            Prefix::TypeInstantiation(type_instantiation) => {
                self.type_instantiation_has_side_effects(type_instantiation, pure_call)
            }
        }
    }

    fn type_instantiation_has_side_effects(
        &self,
        type_instantiation: &TypeInstantiationExpression,
        pure_call: &dyn Fn(&FunctionCall) -> bool,
    ) -> bool {
        self.prefix_has_side_effects(type_instantiation.get_prefix(), pure_call)
    }

    #[inline]
    fn maybe_metatable(&self, value: &LuaValue) -> bool {
        match value {
            LuaValue::False
            | LuaValue::Function
            | LuaValue::Integer(_)
            | LuaValue::Nil
            | LuaValue::Number(_)
            | LuaValue::String(_)
            | LuaValue::Table
            | LuaValue::True => false,
            LuaValue::Unknown => true,
        }
    }

    fn evaluate_binary(&self, expression: &BinaryExpression) -> LuaValue {
        match expression.operator() {
            BinaryOperator::And => self
                .evaluate(expression.left())
                .map_if_truthy(|_| self.evaluate(expression.right())),
            BinaryOperator::Or => self
                .evaluate(expression.left())
                .map_if_truthy_else(|left| left, || self.evaluate(expression.right())),
            BinaryOperator::Equal => self.evaluate_equal(
                &self.evaluate(expression.left()),
                &self.evaluate(expression.right()),
            ),
            BinaryOperator::NotEqual => {
                let result = self.evaluate_equal(
                    &self.evaluate(expression.left()),
                    &self.evaluate(expression.right()),
                );

                match result {
                    LuaValue::True => LuaValue::False,
                    LuaValue::False => LuaValue::True,
                    _ => LuaValue::Unknown,
                }
            }
            BinaryOperator::Plus
            | BinaryOperator::Minus
            | BinaryOperator::Asterisk
            | BinaryOperator::Slash
            | BinaryOperator::DoubleSlash
            | BinaryOperator::Caret
            | BinaryOperator::Percent => self.evaluate_arithmetic(expression),
            BinaryOperator::Concat => {
                match (
                    lua_value::string_coercion_with_subtype(
                        self.evaluate(expression.left()),
                        self.integers,
                    ),
                    lua_value::string_coercion_with_subtype(
                        self.evaluate(expression.right()),
                        self.integers,
                    ),
                ) {
                    (LuaValue::String(mut left), LuaValue::String(right)) => {
                        left.extend_from_slice(&right);
                        LuaValue::String(left)
                    }
                    _ => LuaValue::Unknown,
                }
            }
            BinaryOperator::LowerThan
            | BinaryOperator::LowerOrEqualThan
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterOrEqualThan => self.evaluate_relational(expression),
            BinaryOperator::BitwiseAnd
            | BinaryOperator::BitwiseOr
            | BinaryOperator::BitwiseXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight => Self::bitwise(
                expression.operator(),
                self.evaluate(expression.left()),
                self.evaluate(expression.right()),
            )
            .unwrap_or(LuaValue::Unknown),
        }
    }

    fn evaluate_equal(&self, left: &LuaValue, right: &LuaValue) -> LuaValue {
        match (left, right) {
            (LuaValue::Unknown, _) | (_, LuaValue::Unknown) => LuaValue::Unknown,
            (LuaValue::True, LuaValue::True)
            | (LuaValue::False, LuaValue::False)
            | (LuaValue::Nil, LuaValue::Nil) => LuaValue::True,
            (
                LuaValue::Integer(_) | LuaValue::Number(_),
                LuaValue::Integer(_) | LuaValue::Number(_),
            ) => LuaValue::from(
                Self::compare_numbers(left.clone(), right.clone()) == Some(Ordering::Equal),
            ),
            (LuaValue::String(a), LuaValue::String(b)) => LuaValue::from(a == b),
            _ => LuaValue::False,
        }
    }

    fn evaluate_arithmetic(&self, expression: &BinaryExpression) -> LuaValue {
        let left = self.evaluate(expression.left());
        let right = self.evaluate(expression.right());
        Self::arithmetic(expression.operator(), left, right, self.integers)
            .unwrap_or(LuaValue::Unknown)
    }

    /// Applies an arithmetic operator with Lua 5.3/5.4 number semantics when
    /// `integers` is set: `+`, `-` and `*` stay integers, `/` and `^` produce a
    /// float, and `//` and `%` follow the floor rules. Division or modulo by an
    /// integer zero is an error in Lua and is not folded.
    fn arithmetic(
        operator: BinaryOperator,
        left: LuaValue,
        right: LuaValue,
        integers: bool,
    ) -> Option<LuaValue> {
        let left = lua_value::number_coercion_with_subtype(left, integers);
        let right = lua_value::number_coercion_with_subtype(right, integers);

        if integers {
            if let (LuaValue::Integer(a), LuaValue::Integer(b)) = (&left, &right) {
                let (a, b) = (*a, *b);
                return Some(match operator {
                    BinaryOperator::Plus => LuaValue::Integer(a.wrapping_add(b)),
                    BinaryOperator::Minus => LuaValue::Integer(a.wrapping_sub(b)),
                    BinaryOperator::Asterisk => LuaValue::Integer(a.wrapping_mul(b)),
                    BinaryOperator::Slash => LuaValue::Number(a as f64 / b as f64),
                    BinaryOperator::DoubleSlash if b == 0 => return None,
                    BinaryOperator::DoubleSlash => LuaValue::Integer(floor_division(a, b)),
                    BinaryOperator::Percent if b == 0 => return None,
                    BinaryOperator::Percent => LuaValue::Integer(floor_modulo(a, b)),
                    BinaryOperator::Caret => LuaValue::Number((a as f64).powf(b as f64)),
                    _ => return None,
                });
            }
        }

        let LuaValue::Number(left) = left else {
            return None;
        };
        let LuaValue::Number(right) = right else {
            return None;
        };

        let value = match operator {
            BinaryOperator::Plus => left + right,
            BinaryOperator::Minus => left - right,
            BinaryOperator::Asterisk => left * right,
            BinaryOperator::Slash => left / right,
            BinaryOperator::DoubleSlash => (left / right).floor(),
            BinaryOperator::Caret => left.powf(right),
            BinaryOperator::Percent => left - right * (left / right).floor(),
            _ => return None,
        };
        Some(LuaValue::Number(value))
    }

    fn evaluate_relational(&self, expression: &BinaryExpression) -> LuaValue {
        let left = self.evaluate(expression.left());

        match left {
            LuaValue::Integer(_) | LuaValue::Number(_) => {
                let right = self.evaluate(expression.right());
                Self::compare_numbers(left, right)
                    .map(|ordering| {
                        LuaValue::from(compare_ordering(ordering, expression.operator()))
                    })
                    .unwrap_or(LuaValue::Unknown)
            }
            LuaValue::String(left) => {
                let right = self.evaluate(expression.right());

                if let LuaValue::String(right) = right {
                    self.compare_strings(&left, &right, expression.operator())
                } else {
                    LuaValue::Unknown
                }
            }
            _ => LuaValue::Unknown,
        }
    }

    /// Compares two numbers using Lua 5.3/5.4 rules, comparing integer pairs
    /// exactly and integer/float pairs without losing the integer's precision.
    fn compare_numbers(left: LuaValue, right: LuaValue) -> Option<Ordering> {
        match (left, right) {
            (LuaValue::Integer(a), LuaValue::Integer(b)) => Some(a.cmp(&b)),
            (LuaValue::Integer(a), LuaValue::Number(b)) => compare_integer_and_float(a, b),
            (LuaValue::Number(a), LuaValue::Integer(b)) => {
                compare_integer_and_float(b, a).map(Ordering::reverse)
            }
            (LuaValue::Number(a), LuaValue::Number(b)) => a.partial_cmp(&b),
            _ => None,
        }
    }

    fn compare_strings(&self, left: &[u8], right: &[u8], operator: BinaryOperator) -> LuaValue {
        LuaValue::from(match operator {
            BinaryOperator::Equal => left == right,
            BinaryOperator::NotEqual => left != right,
            BinaryOperator::LowerThan => left < right,
            BinaryOperator::LowerOrEqualThan => left <= right,
            BinaryOperator::GreaterThan => left > right,
            BinaryOperator::GreaterOrEqualThan => left >= right,
            _ => return LuaValue::Unknown,
        })
    }

    fn evaluate_unary(&self, expression: &UnaryExpression) -> LuaValue {
        match expression.operator() {
            UnaryOperator::Not => self
                .evaluate(expression.get_expression())
                .is_truthy()
                .map(|value| LuaValue::from(!value))
                .unwrap_or(LuaValue::Unknown),
            UnaryOperator::Minus => {
                match self.evaluate(expression.get_expression()).number_coercion() {
                    LuaValue::Integer(value) => LuaValue::Integer(value.wrapping_neg()),
                    LuaValue::Number(value) => LuaValue::Number(-value),
                    _ => LuaValue::Unknown,
                }
            }
            UnaryOperator::Length => self.length(self.evaluate(expression.get_expression())),
            UnaryOperator::BitwiseNot => {
                Self::bitwise_not(self.evaluate(expression.get_expression()))
                    .unwrap_or(LuaValue::Unknown)
            }
        }
    }

    fn evaluate_if(&self, expression: &IfExpression) -> LuaValue {
        let condition = self.evaluate(expression.get_condition());

        if let Some(truthy) = condition.is_truthy() {
            if truthy {
                self.evaluate(expression.get_result())
            } else {
                for branch in expression.iter_branches() {
                    let branch_condition = self.evaluate(branch.get_condition());
                    if let Some(truthy) = branch_condition.is_truthy() {
                        if truthy {
                            return self.evaluate(branch.get_result());
                        }
                    } else {
                        return LuaValue::Unknown;
                    }
                }

                self.evaluate(expression.get_else_result())
            }
        } else {
            LuaValue::Unknown
        }
    }

    /// Evaluates an expression against a strict identifier environment.
    ///
    /// Returns `None` when the expression cannot be reduced to a concrete
    /// [`LuaValue`]: an identifier that `lookup` does not provide, a call, a
    /// field access, an index, `...`, or any operation over such a value.
    /// Unlike [`Evaluator::evaluate`], there is no `LuaValue::Unknown` and no
    /// fallback: a missing environment entry propagates as `None`.
    pub fn evaluate_with<F>(&self, expression: &Expression, lookup: &F) -> Option<LuaValue>
    where
        F: Fn(&Identifier) -> Option<LuaValue>,
    {
        match expression {
            Expression::False(_) => Some(LuaValue::False),
            Expression::Function(_) => Some(LuaValue::Function),
            Expression::Nil(_) => Some(LuaValue::Nil),
            Expression::Number(number) => Some(self.number_value(number)),
            Expression::String(string) => Some(LuaValue::from(string.get_value())),
            Expression::Table(_) => Some(LuaValue::Table),
            Expression::True(_) => Some(LuaValue::True),
            Expression::Binary(binary) => self.evaluate_binary_with(binary, lookup),
            Expression::Unary(unary) => self.evaluate_unary_with(unary, lookup),
            Expression::Parenthese(parenthese) => {
                self.evaluate_with(parenthese.inner_expression(), lookup)
            }
            Expression::If(if_expression) => self.evaluate_if_with(if_expression, lookup),
            Expression::InterpolatedString(string) => {
                self.evaluate_interpolated_string_with(string, lookup)
            }
            Expression::TypeCast(type_cast) => {
                self.evaluate_with(type_cast.get_expression(), lookup)
            }
            Expression::TypeInstantiation(type_instantiation) => {
                self.evaluate_type_instantiation_with(type_instantiation, lookup)
            }
            Expression::Identifier(identifier) => lookup(identifier),
            // Calls, field reads, index reads and `...` depend on runtime state
            // that an identifier environment cannot describe.
            Expression::Call(_)
            | Expression::Field(_)
            | Expression::Index(_)
            | Expression::VariableArguments(_) => None,
        }
    }

    fn evaluate_binary_with<F>(&self, expression: &BinaryExpression, lookup: &F) -> Option<LuaValue>
    where
        F: Fn(&Identifier) -> Option<LuaValue>,
    {
        match expression.operator() {
            BinaryOperator::And => {
                let left = self.evaluate_with(expression.left(), lookup)?;
                match left.is_truthy()? {
                    true => self.evaluate_with(expression.right(), lookup),
                    false => Some(left),
                }
            }
            BinaryOperator::Or => {
                let left = self.evaluate_with(expression.left(), lookup)?;
                match left.is_truthy()? {
                    true => Some(left),
                    false => self.evaluate_with(expression.right(), lookup),
                }
            }
            BinaryOperator::Equal => {
                let left = self.evaluate_with(expression.left(), lookup)?;
                let right = self.evaluate_with(expression.right(), lookup)?;
                match self.evaluate_equal(&left, &right) {
                    LuaValue::Unknown => None,
                    value => Some(value),
                }
            }
            BinaryOperator::NotEqual => {
                let left = self.evaluate_with(expression.left(), lookup)?;
                let right = self.evaluate_with(expression.right(), lookup)?;
                match self.evaluate_equal(&left, &right) {
                    LuaValue::True => Some(LuaValue::False),
                    LuaValue::False => Some(LuaValue::True),
                    _ => None,
                }
            }
            BinaryOperator::Concat => {
                let left = self.evaluate_with(expression.left(), lookup)?;
                let right = self.evaluate_with(expression.right(), lookup)?;
                Self::concatenate(left, right, self.integers)
            }
            operator => {
                let left = self.evaluate_with(expression.left(), lookup)?;
                let right = self.evaluate_with(expression.right(), lookup)?;
                match operator {
                    BinaryOperator::LowerThan
                    | BinaryOperator::LowerOrEqualThan
                    | BinaryOperator::GreaterThan
                    | BinaryOperator::GreaterOrEqualThan => {
                        self.evaluate_relational_operator(operator, left, right)
                    }
                    BinaryOperator::BitwiseAnd
                    | BinaryOperator::BitwiseOr
                    | BinaryOperator::BitwiseXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight => Self::bitwise(operator, left, right),
                    _ => Self::arithmetic(operator, left, right, self.integers),
                }
            }
        }
    }

    /// Folds a Lua 5.3 bitwise operator over integer-valued operands.
    ///
    /// Returns `None` when an operand is not an integer, so the folder declines
    /// instead of guessing.
    fn bitwise(operator: BinaryOperator, left: LuaValue, right: LuaValue) -> Option<LuaValue> {
        let left = Self::to_integer(left)?;
        let right = Self::to_integer(right)?;
        let value = match operator {
            BinaryOperator::BitwiseAnd => left & right,
            BinaryOperator::BitwiseOr => left | right,
            BinaryOperator::BitwiseXor => left ^ right,
            BinaryOperator::ShiftLeft => Self::shift(left, right, true),
            BinaryOperator::ShiftRight => Self::shift(left, right, false),
            _ => return None,
        };
        Some(LuaValue::Integer(value))
    }

    fn bitwise_not(value: LuaValue) -> Option<LuaValue> {
        Some(LuaValue::Integer(!Self::to_integer(value)?))
    }

    fn to_integer(value: LuaValue) -> Option<i64> {
        match value.number_coercion() {
            LuaValue::Integer(value) => Some(value),
            LuaValue::Number(value) => {
                // `i64::MAX as f64` rounds up to 2^63, so the bound must be
                // exclusive: a value equal to 2^63 does not fit a Lua integer.
                if !value.is_finite()
                    || value.fract() != 0.0
                    || value < i64::MIN as f64
                    || value >= i64::MAX as f64
                {
                    return None;
                }
                Some(value as i64)
            }
            _ => None,
        }
    }

    /// Implements the Lua 5.3 shift rules: a negative shift reverses direction
    /// and shifts of 64 bits or more produce zero.
    fn shift(value: i64, amount: i64, left: bool) -> i64 {
        let (value, amount, left) = if amount < 0 {
            (value, amount.unsigned_abs(), !left)
        } else {
            (value, amount as u64, left)
        };
        if amount >= 64 {
            return 0;
        }
        if left {
            ((value as u64) << amount) as i64
        } else {
            ((value as u64) >> amount) as i64
        }
    }

    fn evaluate_relational_operator(
        &self,
        operator: BinaryOperator,
        left: LuaValue,
        right: LuaValue,
    ) -> Option<LuaValue> {
        match (&left, &right) {
            (
                LuaValue::Integer(_) | LuaValue::Number(_),
                LuaValue::Integer(_) | LuaValue::Number(_),
            ) => Self::compare_numbers(left, right)
                .map(|ordering| LuaValue::from(compare_ordering(ordering, operator))),
            (LuaValue::String(left), LuaValue::String(right)) => {
                match self.compare_strings(left, right, operator) {
                    LuaValue::Unknown => None,
                    value => Some(value),
                }
            }
            _ => None,
        }
    }

    fn concatenate(left: LuaValue, right: LuaValue, integers: bool) -> Option<LuaValue> {
        match (
            lua_value::string_coercion_with_subtype(left, integers),
            lua_value::string_coercion_with_subtype(right, integers),
        ) {
            (LuaValue::String(mut left), LuaValue::String(right)) => {
                left.extend_from_slice(&right);
                Some(LuaValue::String(left))
            }
            _ => None,
        }
    }

    /// Folds the `#` operator.
    ///
    /// `#` produces an integer in Lua 5.3/5.4 and a float in 5.1/5.2/Luau, so
    /// the result subtype follows the evaluator's number model.
    fn length(&self, value: LuaValue) -> LuaValue {
        match value.length() {
            LuaValue::Number(value) if self.integer_numbers() => LuaValue::Integer(value as i64),
            length => length,
        }
    }

    fn evaluate_unary_with<F>(&self, expression: &UnaryExpression, lookup: &F) -> Option<LuaValue>
    where
        F: Fn(&Identifier) -> Option<LuaValue>,
    {
        let value = self.evaluate_with(expression.get_expression(), lookup)?;

        match expression.operator() {
            UnaryOperator::Not => Some(LuaValue::from(!value.is_truthy()?)),
            UnaryOperator::Minus => match value.number_coercion() {
                LuaValue::Integer(number) => Some(LuaValue::Integer(number.wrapping_neg())),
                LuaValue::Number(number) => Some(LuaValue::from(-number)),
                _ => None,
            },
            UnaryOperator::Length => match self.length(value) {
                LuaValue::Unknown => None,
                length => Some(length),
            },
            UnaryOperator::BitwiseNot => Self::bitwise_not(value),
        }
    }

    fn evaluate_if_with<F>(&self, expression: &IfExpression, lookup: &F) -> Option<LuaValue>
    where
        F: Fn(&Identifier) -> Option<LuaValue>,
    {
        match self
            .evaluate_with(expression.get_condition(), lookup)?
            .is_truthy()?
        {
            true => self.evaluate_with(expression.get_result(), lookup),
            false => {
                for branch in expression.iter_branches() {
                    let truthy = self
                        .evaluate_with(branch.get_condition(), lookup)?
                        .is_truthy()?;
                    if truthy {
                        return self.evaluate_with(branch.get_result(), lookup);
                    }
                }

                self.evaluate_with(expression.get_else_result(), lookup)
            }
        }
    }

    fn evaluate_interpolated_string_with<F>(
        &self,
        expression: &InterpolatedStringExpression,
        lookup: &F,
    ) -> Option<LuaValue>
    where
        F: Fn(&Identifier) -> Option<LuaValue>,
    {
        let mut result = Vec::new();

        for segment in expression.iter_segments() {
            match segment {
                InterpolationSegment::String(string) => {
                    result.extend_from_slice(string.get_value());
                }
                InterpolationSegment::Value(value) => {
                    match self.evaluate_with(value.get_expression(), lookup)? {
                        LuaValue::False => result.extend_from_slice(b"false"),
                        LuaValue::True => result.extend_from_slice(b"true"),
                        LuaValue::Nil => result.extend_from_slice(b"nil"),
                        LuaValue::String(string) => result.extend_from_slice(&string),
                        // Matches `Evaluator::evaluate`, which cannot produce a
                        // textual form for these values.
                        LuaValue::Function
                        | LuaValue::Integer(_)
                        | LuaValue::Number(_)
                        | LuaValue::Table
                        | LuaValue::Unknown => return None,
                    }
                }
            }
        }

        Some(LuaValue::String(result))
    }

    fn evaluate_type_instantiation_with<F>(
        &self,
        expression: &TypeInstantiationExpression,
        lookup: &F,
    ) -> Option<LuaValue>
    where
        F: Fn(&Identifier) -> Option<LuaValue>,
    {
        self.evaluate_prefix_with(expression.get_prefix(), lookup)
    }

    fn evaluate_prefix_with<F>(&self, prefix: &Prefix, lookup: &F) -> Option<LuaValue>
    where
        F: Fn(&Identifier) -> Option<LuaValue>,
    {
        match prefix {
            Prefix::Parenthese(parenthese) => {
                self.evaluate_with(parenthese.inner_expression(), lookup)
            }
            Prefix::Identifier(identifier) => lookup(identifier),
            Prefix::TypeInstantiation(type_instantiation) => {
                self.evaluate_type_instantiation_with(type_instantiation, lookup)
            }
            Prefix::Call(_) | Prefix::Field(_) | Prefix::Index(_) => None,
        }
    }
}

/// Compares an integer with a float without rounding the integer through `f64`.
fn compare_integer_and_float(integer: i64, float: f64) -> Option<Ordering> {
    if float.is_nan() {
        return None;
    }
    if float >= i64::MAX as f64 {
        return Some(Ordering::Less);
    }
    if float < i64::MIN as f64 {
        return Some(Ordering::Greater);
    }
    let floored = float.floor();
    let floored_integer = floored as i64;
    Some(match integer.cmp(&floored_integer) {
        Ordering::Equal if float > floored => Ordering::Less,
        ordering => ordering,
    })
}

/// Maps a numeric ordering to the result of a relational operator.
fn compare_ordering(ordering: Ordering, operator: BinaryOperator) -> bool {
    match operator {
        BinaryOperator::LowerThan => ordering == Ordering::Less,
        BinaryOperator::LowerOrEqualThan => ordering != Ordering::Greater,
        BinaryOperator::GreaterThan => ordering == Ordering::Greater,
        BinaryOperator::GreaterOrEqualThan => ordering != Ordering::Less,
        _ => false,
    }
}

/// Lua 5.3/5.4 floor division for integers.
fn floor_division(left: i64, right: i64) -> i64 {
    let quotient = left.wrapping_div(right);
    let remainder = left.wrapping_rem(right);
    if remainder != 0 && ((remainder < 0) != (right < 0)) {
        quotient.wrapping_sub(1)
    } else {
        quotient
    }
}

/// Lua 5.3/5.4 floor modulo for integers.
fn floor_modulo(left: i64, right: i64) -> i64 {
    let remainder = left.wrapping_rem(right);
    if remainder != 0 && ((remainder < 0) != (right < 0)) {
        remainder.wrapping_add(right)
    } else {
        remainder
    }
}

#[cfg(test)]
mod test {
    use super::*;

    macro_rules! evaluate_expressions {
        ($($name:ident ($expression:expr) => $value:expr),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    assert_eq!($value, Evaluator::default().evaluate(&$expression.into()));
                }
            )*
        };
    }

    #[test]
    fn the_length_operator_follows_the_number_model() {
        let length = || {
            Expression::from(UnaryExpression::new(
                UnaryOperator::Length,
                Expression::from(StringExpression::from_value("abc")),
            ))
        };

        assert_eq!(
            Evaluator::default().evaluate(&length()),
            LuaValue::Number(3.0)
        );
        assert_eq!(
            Evaluator::default()
                .with_integer_numbers()
                .evaluate(&length()),
            LuaValue::Integer(3)
        );
    }

    #[test]
    fn evaluate_with_resolves_identifiers_strictly() {
        let evaluator = Evaluator::default();
        let lookup = |identifier: &Identifier| match identifier.get_name().as_str() {
            "a" => Some(LuaValue::Number(1.0)),
            "flag" => Some(LuaValue::False),
            _ => None,
        };

        let sum = BinaryExpression::new(
            BinaryOperator::Plus,
            Expression::identifier("a"),
            Expression::from(2),
        );
        assert_eq!(
            evaluator.evaluate_with(&sum.into(), &lookup),
            Some(LuaValue::Number(3.0))
        );

        let missing = Expression::identifier("missing");
        assert_eq!(evaluator.evaluate_with(&missing, &lookup), None);

        let and = BinaryExpression::new(
            BinaryOperator::And,
            Expression::identifier("flag"),
            Expression::identifier("missing"),
        );
        assert_eq!(
            evaluator.evaluate_with(&and.into(), &lookup),
            Some(LuaValue::False),
            "a falsy left operand short-circuits without the right operand"
        );

        let or = BinaryExpression::new(
            BinaryOperator::Or,
            Expression::identifier("a"),
            Expression::identifier("missing"),
        );
        assert_eq!(
            evaluator.evaluate_with(&or.into(), &lookup),
            Some(LuaValue::Number(1.0)),
            "a truthy left operand short-circuits without the right operand"
        );

        let call = FunctionCall::from_name("f");
        assert_eq!(evaluator.evaluate_with(&call.into(), &lookup), None);
    }

    #[test]
    fn evaluate_with_requires_if_conditions() {
        let evaluator = Evaluator::default();
        let expression = IfExpression::new(Expression::identifier("condition"), 1.0, 2.0).into();

        let truthy = |_: &Identifier| Some(LuaValue::True);
        let falsy = |_: &Identifier| Some(LuaValue::False);
        let unknown = |_: &Identifier| None;

        assert_eq!(
            evaluator.evaluate_with(&expression, &truthy),
            Some(LuaValue::Number(1.0))
        );
        assert_eq!(
            evaluator.evaluate_with(&expression, &falsy),
            Some(LuaValue::Number(2.0))
        );
        assert_eq!(evaluator.evaluate_with(&expression, &unknown), None);
    }

    #[test]
    fn evaluate_with_matches_evaluate_for_closed_expressions() {
        let evaluator = Evaluator::default();
        let no_environment = |_: &Identifier| None;

        let cases: Vec<Expression> = vec![
            Expression::from(true),
            Expression::from(false),
            Expression::nil(),
            DecimalNumber::new(4.0).into(),
            StringExpression::from_value("x").into(),
            BinaryExpression::new(
                BinaryOperator::Plus,
                DecimalNumber::new(1.0),
                DecimalNumber::new(2.0),
            )
            .into(),
            UnaryExpression::new(UnaryOperator::Not, false).into(),
            IfExpression::new(true, 1.0, 2.0).into(),
        ];

        for expression in cases {
            assert_eq!(
                evaluator.evaluate_with(&expression, &no_environment),
                Some(evaluator.evaluate(&expression)),
                "closed expression must evaluate identically: {expression:?}"
            );
        }
    }

    #[test]
    fn only_calls_and_variable_arguments_return_multiple_values() {
        let evaluator = Evaluator::default();

        assert!(evaluator.can_return_multiple_values(&FunctionCall::from_name("f").into()));
        assert!(evaluator.can_return_multiple_values(&Expression::variable_arguments()));

        let unary = UnaryExpression::new(UnaryOperator::Not, Expression::variable_arguments());
        assert!(
            !evaluator.can_return_multiple_values(&unary.into()),
            "unary operators adjust their operand to a single value"
        );

        for operator in [BinaryOperator::Plus, BinaryOperator::And] {
            let binary = BinaryExpression::new(
                operator,
                Expression::variable_arguments(),
                Expression::from(1),
            );
            assert!(
                !evaluator.can_return_multiple_values(&binary.into()),
                "binary operators always return a single value"
            );
        }
    }

    evaluate_expressions!(
        true_expression(Expression::from(true)) => LuaValue::True,
        false_expression(Expression::from(false)) => LuaValue::False,
        nil_expression(Expression::nil()) => LuaValue::Nil,
        number_expression(DecimalNumber::new(0.0)) => LuaValue::Number(0.0),
        number_expression_negative_zero(DecimalNumber::new(-0.0)) => LuaValue::Number(-0.0),
        string_expression(StringExpression::from_value("foo")) => LuaValue::from("foo"),
        empty_interpolated_string_expression(InterpolatedStringExpression::empty()) => LuaValue::from(""),
        interpolated_string_expression_with_one_string(InterpolatedStringExpression::empty().with_segment("hello"))
            => LuaValue::from("hello"),
        interpolated_string_expression_with_multiple_string_segments(
            InterpolatedStringExpression::empty()
                .with_segment("hello")
                .with_segment("-")
                .with_segment("bye")
        ) => LuaValue::from("hello-bye"),
        interpolated_string_expression_with_true_segment(
            InterpolatedStringExpression::empty().with_segment(Expression::from(true))
        ) => LuaValue::from("true"),
        interpolated_string_expression_with_false_segment(
            InterpolatedStringExpression::empty().with_segment(Expression::from(false))
        ) => LuaValue::from("false"),
        interpolated_string_expression_with_nil_segment(
            InterpolatedStringExpression::empty().with_segment(Expression::nil())
        ) => LuaValue::from("nil"),
        interpolated_string_expression_with_mixed_segments(
            InterpolatedStringExpression::empty()
                .with_segment("variable = ")
                .with_segment(Expression::from(true))
                .with_segment("?")
        ) => LuaValue::from("variable = true?"),
        interpolated_string_expression_with_mixed_segments_unknown(
            InterpolatedStringExpression::empty()
                .with_segment("variable = ")
                .with_segment(Expression::identifier("test"))
                .with_segment("!")
        ) => LuaValue::Unknown,
        true_wrapped_in_parens(ParentheseExpression::new(true)) => LuaValue::True,
        false_wrapped_in_parens(ParentheseExpression::new(false)) => LuaValue::False,
        nil_wrapped_in_parens(ParentheseExpression::new(Expression::nil())) => LuaValue::Nil,
        number_wrapped_in_parens(ParentheseExpression::new(DecimalNumber::new(0.0)))
            => LuaValue::Number(0.0),
        string_wrapped_in_parens(ParentheseExpression::new(StringExpression::from_value("foo")))
            => LuaValue::from("foo"),
        table_expression(TableExpression::default()) => LuaValue::Table,
        if_expression_always_true(IfExpression::new(true, 1.0, 0.0)) => LuaValue::from(1.0),
        if_expression_always_false(IfExpression::new(false, 1.0, 0.0)) => LuaValue::from(0.0),
        if_expression_unknown_condition(IfExpression::new(Expression::identifier("test"), 1.0, 0.0))
            => LuaValue::Unknown,
        if_expression_elseif_always_true(IfExpression::new(false, 1.0, 0.0).with_branch(true, 2.0))
            => LuaValue::from(2.0),
        if_expression_elseif_always_false(IfExpression::new(false, 1.0, 0.0).with_branch(false, 2.0))
            => LuaValue::from(0.0),
        length_empty_string(UnaryExpression::new(UnaryOperator::Length, StringExpression::empty()))
            => LuaValue::Number(0.0),
        length_single_char_string(UnaryExpression::new(UnaryOperator::Length, StringExpression::from_value("a")))
            => LuaValue::Number(1.0),
        length_short_string(UnaryExpression::new(UnaryOperator::Length, StringExpression::from_value("hello")))
            => LuaValue::Number(5.0),
        length_unknown_expression(UnaryExpression::new(UnaryOperator::Length, Expression::identifier("var")))
            => LuaValue::Unknown,
        length_number_expression(UnaryExpression::new(UnaryOperator::Length, Expression::from(42.0)))
            => LuaValue::Unknown,
        length_nil_expression(UnaryExpression::new(UnaryOperator::Length, Expression::nil()))
            => LuaValue::Unknown,
    );

    mod binary_expressions {
        use super::*;

        macro_rules! evaluate_binary_expressions {
            ($($name:ident ($operator:expr, $left:expr, $right:expr) => $expect:expr),* $(,)?) => {
                $(
                    #[test]
                    fn $name() {
                        let binary = BinaryExpression::new($operator, $left, $right);

                        let result = Evaluator::default().evaluate(&binary.into());

                        match (&$expect, &result) {
                            (LuaValue::Number(expect_float), LuaValue::Number(result))=> {
                                if expect_float.is_nan() {
                                    assert!(result.is_nan(), "{} should be NaN", result);
                                } else if expect_float.is_infinite() {
                                    assert!(result.is_infinite(), "{} should be infinite", result);
                                    assert_eq!(expect_float.is_sign_positive(), result.is_sign_positive());
                                } else {
                                    assert!(
                                        (expect_float - result).abs() < f64::EPSILON,
                                        "{} does not approximate {}", result, expect_float
                                    );
                                    assert!(
                                        expect_float.is_sign_positive() == result.is_sign_positive(),
                                        "{} should be of the same sign as {}", result, expect_float
                                    );
                                }
                            }
                            _ => {
                                assert_eq!($expect, result);
                            }
                        }
                    }
                )*
            };
        }

        evaluate_binary_expressions!(
            true_and_number(
                BinaryOperator::And,
                true,
                Expression::Number(DecimalNumber::new(0.0).into())
            ) => LuaValue::Number(0.0),
            true_and_true(
                BinaryOperator::And,
                true,
                true
            ) => LuaValue::True,
            true_and_false(
                BinaryOperator::And,
                true,
                false
            ) => LuaValue::False,
            true_and_nil(
                BinaryOperator::And,
                true,
                Expression::nil()
            ) => LuaValue::Nil,
            true_and_string(
                BinaryOperator::And,
                true,
                Expression::String(StringExpression::from_value("foo"))
            ) => LuaValue::from("foo"),
            true_and_table(
                BinaryOperator::And,
                true,
                TableExpression::default()
            ) => LuaValue::Table,
            nil_and_true(
                BinaryOperator::And,
                Expression::nil(),
                true
            ) => LuaValue::Nil,
            false_and_true(
                BinaryOperator::And,
                false,
                true
            ) => LuaValue::False,
            true_or_number(
                BinaryOperator::Or,
                true,
                Expression::Number(DecimalNumber::new(0.0).into())
            ) => LuaValue::True,
            true_or_true(
                BinaryOperator::Or,
                true,
                true
            ) => LuaValue::True,
            true_or_false(
                BinaryOperator::Or,
                true,
                false
            ) => LuaValue::True,
            true_or_nil(
                BinaryOperator::Or,
                true,
                Expression::nil()
            ) => LuaValue::True,
            true_or_string(
                BinaryOperator::Or,
                true,
                Expression::String(StringExpression::from_value("foo"))
            ) => LuaValue::True,
            nil_or_true(
                BinaryOperator::Or,
                Expression::nil(),
                true
            ) => LuaValue::True,
            nil_or_false(
                BinaryOperator::Or,
                Expression::nil(),
                false
            ) => LuaValue::False,
            nil_or_nil(
                BinaryOperator::Or,
                Expression::nil(),
                Expression::nil()
            ) => LuaValue::Nil,
            one_plus_two(
                BinaryOperator::Plus,
                Expression::from(1.0),
                Expression::from(2.0)
            ) => LuaValue::Number(3.0),
            one_minus_two(
                BinaryOperator::Minus,
                Expression::from(1.0),
                Expression::from(2.0)
            ) => LuaValue::Number(-1.0),
            three_times_four(
                BinaryOperator::Asterisk,
                Expression::from(3.0),
                Expression::from(4.0)
            ) => LuaValue::Number(12.0),
            twelve_divided_by_four(
                BinaryOperator::Slash,
                Expression::from(12.0),
                Expression::from(4.0)
            ) => LuaValue::Number(3.0),
            one_divided_by_zero(
                BinaryOperator::Slash,
                Expression::from(1.0),
                Expression::from(0.0)
            ) => LuaValue::Number(f64::INFINITY),
            negative_zero_plus_negative_zero(
                BinaryOperator::Plus,
                Expression::from(-0.0),
                Expression::from(-0.0)
            ) => LuaValue::Number(-0.0),
            negative_zero_minus_zero(
                BinaryOperator::Minus,
                Expression::from(-0.0),
                Expression::from(0.0)
            ) => LuaValue::Number(-0.0),
            zero_divided_by_zero(
                BinaryOperator::Slash,
                Expression::from(0.0),
                Expression::from(0.0)
            ) => LuaValue::Number(f64::NAN),
            twelve_floor_division_by_four(
                BinaryOperator::DoubleSlash,
                Expression::from(12.0),
                Expression::from(4.0)
            ) => LuaValue::Number(3.0),
            eleven_floor_division_by_three(
                BinaryOperator::DoubleSlash,
                Expression::from(11.0),
                Expression::from(3.0)
            ) => LuaValue::Number(3.0),
            one_floor_division_by_zero(
                BinaryOperator::DoubleSlash,
                Expression::from(1.0),
                Expression::from(0.0)
            ) => LuaValue::Number(f64::INFINITY),
            minus_one_floor_division_by_zero(
                BinaryOperator::DoubleSlash,
                Expression::from(-1.0),
                Expression::from(0.0)
            ) => LuaValue::Number(f64::NEG_INFINITY),
            zero_floor_division_by_zero(
                BinaryOperator::DoubleSlash,
                Expression::from(0.0),
                Expression::from(0.0)
            ) => LuaValue::Number(f64::NAN),
            five_mod_two(
                BinaryOperator::Percent,
                Expression::from(5.0),
                Expression::from(2.0)
            ) => LuaValue::Number(1.0),
            minus_five_mod_two(
                BinaryOperator::Percent,
                Expression::from(-5.0),
                Expression::from(2.0)
            ) => LuaValue::Number(1.0),
            minus_five_mod_minus_two(
                BinaryOperator::Percent,
                Expression::from(-5.0),
                Expression::from(-2.0)
            ) => LuaValue::Number(-1.0),
            five_point_two_mod_two(
                BinaryOperator::Percent,
                Expression::from(5.5),
                Expression::from(2.0)
            ) => LuaValue::Number(1.5),
            five_pow_two(
                BinaryOperator::Caret,
                Expression::from(5.0),
                Expression::from(2.0)
            ) => LuaValue::Number(25.0),
            string_number_plus_string_number(
                BinaryOperator::Plus,
                StringExpression::from_value("2"),
                StringExpression::from_value("3")
            ) => LuaValue::Number(5.0),
            concat_strings(
                BinaryOperator::Concat,
                StringExpression::from_value("2"),
                StringExpression::from_value("3")
            ) => LuaValue::from("23"),
            concat_string_with_number(
                BinaryOperator::Concat,
                StringExpression::from_value("foo"),
                11.0
            ) => LuaValue::from("foo11"),
            concat_number_with_string(
                BinaryOperator::Concat,
                11.0,
                StringExpression::from_value("foo")
            ) => LuaValue::from("11foo"),
            concat_number_with_number(
                BinaryOperator::Concat,
                11.0,
                33.0
            ) => LuaValue::from("1133"),
            concat_number_with_negative_number(
                BinaryOperator::Concat,
                11.0,
                -33.0
            ) => LuaValue::from("11-33"),
            concat_empty_strings(
                BinaryOperator::Concat,
                StringExpression::empty(),
                StringExpression::empty()
            ) => LuaValue::from(""),
            number_lower_than_string(
                BinaryOperator::LowerThan,
                1.0,
                StringExpression::empty()
            ) => LuaValue::Unknown,
            number_string_greater_than_number(
                BinaryOperator::GreaterThan,
                StringExpression::from_value("100"),
                1.0
            ) => LuaValue::Unknown,
            number_string_greater_or_equal_than_number(
                BinaryOperator::GreaterOrEqualThan,
                StringExpression::from_value("100"),
                100.0
            ) => LuaValue::Unknown,
            number_lower_or_equal_than_number_string(
                BinaryOperator::GreaterOrEqualThan,
                100.0,
                StringExpression::from_value("100")
            ) => LuaValue::Unknown,
        );

        macro_rules! evaluate_equality {
            ($($name:ident ($left:expr, $right:expr) => $value:expr),* $(,)?) => {
                $(
                    mod $name {
                        use super::*;

                        #[test]
                        fn equal() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::Equal,
                                $left,
                                $right,
                            );

                            assert_eq!($value, Evaluator::default().evaluate(&binary.into()));

                            let binary = BinaryExpression::new(
                                BinaryOperator::Equal,
                                $right,
                                $left,
                            );

                            assert_eq!($value, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn not_equal() {
                            let value = match $value {
                                LuaValue::True => LuaValue::False,
                                LuaValue::False => LuaValue::True,
                                _ => LuaValue::Unknown
                            };
                            let binary = BinaryExpression::new(
                                BinaryOperator::NotEqual,
                                $left,
                                $right,
                            );

                            assert_eq!(value, Evaluator::default().evaluate(&binary.into()));

                            let binary = BinaryExpression::new(
                                BinaryOperator::NotEqual,
                                $right,
                                $left,
                            );

                            assert_eq!(value, Evaluator::default().evaluate(&binary.into()));
                        }
                    }
                )*
            };
        }

        evaluate_equality!(
            true_true(Expression::from(true), Expression::from(true)) => LuaValue::True,
            false_false(Expression::from(false), Expression::from(false)) => LuaValue::True,
            nil_nil(Expression::nil(), Expression::nil()) => LuaValue::True,
            same_strings(
                StringExpression::from_value("foo"),
                StringExpression::from_value("foo")
            ) => LuaValue::True,
            same_numbers(
                Expression::Number(DecimalNumber::new(0.0).into()),
                Expression::Number(DecimalNumber::new(0.0).into())
            ) => LuaValue::True,
            true_false(Expression::from(true), Expression::from(false)) => LuaValue::False,
            true_nil(Expression::from(true), Expression::from(false)) => LuaValue::False,
            different_numbers(
                Expression::Number(DecimalNumber::new(1.0).into()),
                Expression::Number(DecimalNumber::new(10.0).into())
            ) => LuaValue::False,
            different_strings(
                StringExpression::from_value("foo"),
                StringExpression::from_value("bar")
            ) => LuaValue::False,
        );

        macro_rules! evaluate_equality_with_relational_operators {
            ($($name:ident => $value:expr),* $(,)?) => {
                $(
                    mod $name {
                        use super::*;

                        #[test]
                        fn lower() {
                            let value: Expression = $value.into();
                            let binary = BinaryExpression::new(BinaryOperator::LowerThan, value.clone(), value);
                            assert_eq!(LuaValue::False, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn lower_or_equal() {
                            let value: Expression = $value.into();
                            let binary = BinaryExpression::new(BinaryOperator::LowerOrEqualThan, value.clone(), value);
                            assert_eq!(LuaValue::True, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn greater() {
                            let value: Expression = $value.into();
                            let binary = BinaryExpression::new(BinaryOperator::GreaterThan, value.clone(), value);
                            assert_eq!(LuaValue::False, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn greater_or_equal() {
                            let value: Expression = $value.into();
                            let binary = BinaryExpression::new(BinaryOperator::GreaterOrEqualThan, value.clone(), value);
                            assert_eq!(LuaValue::True, Evaluator::default().evaluate(&binary.into()));
                        }
                    }
                )*
            };
        }

        evaluate_equality_with_relational_operators!(
            zero => 1.0,
            one => 1.0,
            hundred => 100.0,
            string => StringExpression::from_value("var"),
        );

        macro_rules! evaluate_strict_relational_operators {
            ($($name_lower:ident($lower:expr) < $name_greater:ident($greater:expr)),* $(,)?) => {
                mod lower_or_greater_than {
                    use super::*;
                    paste::paste! {

                    $(
                        #[test]
                        fn [<$name_lower _lower_than_ $name_greater>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::LowerThan,
                                $lower,
                                $greater,
                            );
                            assert_eq!(LuaValue::True, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn [<$name_lower _lower_or_equal_than_ $name_greater>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::LowerOrEqualThan,
                                $lower,
                                $greater,
                            );
                            assert_eq!(LuaValue::True, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn [<$name_lower _greater_than_ $name_greater>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::GreaterThan,
                                $lower,
                                $greater,
                            );
                            assert_eq!(LuaValue::False, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn [<$name_lower _greater_or_equal_than_ $name_greater>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::GreaterOrEqualThan,
                                $lower,
                                $greater,
                            );
                            assert_eq!(LuaValue::False, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn [<$name_greater _lower_than_ $name_lower>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::LowerThan,
                                $greater,
                                $lower,
                            );
                            assert_eq!(LuaValue::False, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn [<$name_greater _lower_or_equal_than_ $name_lower>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::LowerOrEqualThan,
                                $greater,
                                $lower,
                            );
                            assert_eq!(LuaValue::False, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn [<$name_greater _greater_than_ $name_lower>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::GreaterThan,
                                $greater,
                                $lower,
                            );
                            assert_eq!(LuaValue::True, Evaluator::default().evaluate(&binary.into()));
                        }

                        #[test]
                        fn [<$name_greater _greater_or_equal_than_ $name_lower>]() {
                            let binary = BinaryExpression::new(
                                BinaryOperator::GreaterOrEqualThan,
                                $greater,
                                $lower,
                            );
                            assert_eq!(LuaValue::True, Evaluator::default().evaluate(&binary.into()));
                        }
                    )*

                    }
                }
            };
        }

        evaluate_strict_relational_operators!(
            one(1.0) < hundred(100.0),
            minus_15(-15.0) < minus_2_5(-2.5),
            string_a(StringExpression::from_value("a"))
                < string_b(StringExpression::from_value("b")),
            string_a(StringExpression::from_value("a"))
                < string_aa(StringExpression::from_value("aa")),
            string_1(StringExpression::from_value("1"))
                < string_a(StringExpression::from_value("a")),
            string_111(StringExpression::from_value("111"))
                < string_a(StringExpression::from_value("a")),
            empty_string(StringExpression::from_value(""))
                < string_colon(StringExpression::from_value(":")),
        );
    }

    mod unary_expressions {
        use super::*;
        use UnaryOperator::*;

        macro_rules! evaluate_unary_expressions {
            ($($name:ident ($operator:expr, $input:expr) => $value:expr),*) => {
                $(
                    #[test]
                    fn $name() {
                        let unary = UnaryExpression::new($operator, $input);
                        assert_eq!($value, Evaluator::default().evaluate(&unary.into()));
                    }
                )*
            };
        }

        evaluate_unary_expressions!(
            not_true(Not, Expression::from(true)) => LuaValue::False,
            not_false(Not, Expression::from(false)) => LuaValue::True,
            not_nil(Not, Expression::nil()) => LuaValue::True,
            not_table(Not, TableExpression::default()) => LuaValue::False,
            not_string(Not, StringExpression::from_value("foo")) => LuaValue::False,
            not_number(
                Not,
                Expression::Number(DecimalNumber::new(10.0).into())
            ) => LuaValue::False,
            not_identifier(Not, Expression::identifier("foo")) => LuaValue::Unknown,
            minus_one(Minus, DecimalNumber::new(1.0)) => LuaValue::from(-1.0),
            minus_zero(Minus, DecimalNumber::new(-0.0)) => LuaValue::from(-0.0),
            minus_negative_number(Minus, DecimalNumber::new(-5.0)) => LuaValue::from(5.0),
            minus_string_converted_to_number(Minus, StringExpression::from_value("1")) => LuaValue::from(-1.0)
        );
    }

    macro_rules! has_side_effects {
        ($($name:ident => $expression:expr),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    assert!(Evaluator::default().has_side_effects(&$expression.into()));
                }
            )*
        };
    }

    macro_rules! has_no_side_effects {
        ($($name:ident => $expression:expr),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    assert!(!Evaluator::default().has_side_effects(&$expression.into()));
                }
            )*
        };
    }

    has_side_effects!(
        call_to_unknown_function => FunctionCall::from_name("foo"),
        binary_true_and_call => BinaryExpression::new(
            BinaryOperator::And,
            Expression::from(true),
            FunctionCall::from_name("foo"),
        ),
        binary_false_or_call => BinaryExpression::new(
            BinaryOperator::Or,
            Expression::from(false),
            FunctionCall::from_name("var"),
        ),
        addition_unknown_variable_and_number => BinaryExpression::new(
            BinaryOperator::Plus,
            Expression::identifier("var"),
            1.0,
        ),
        addition_number_with_unknown_variable => BinaryExpression::new(
            BinaryOperator::Plus,
            1.0,
            Expression::identifier("var"),
        ),
        unary_minus_on_variable => UnaryExpression::new(UnaryOperator::Minus, Identifier::new("var")),
        length_on_variable => UnaryExpression::new(UnaryOperator::Length, Identifier::new("var")),
        field_index => FieldExpression::new(Identifier::new("var"), "field"),
        table_value_with_call_in_entry => TableExpression::default()
            .append_array_value(FunctionCall::from_name("call")),

        interpolated_string_with_function_call => InterpolatedStringExpression::empty()
            .with_segment(Expression::from(FunctionCall::from_name("foo"))),
    );

    has_no_side_effects!(
        true_value => Expression::from(true),
        false_value => Expression::from(false),
        nil_value => Expression::nil(),
        table_value => TableExpression::default(),
        number_value => Expression::Number(DecimalNumber::new(0.0).into()),
        string_value => StringExpression::from_value(""),
        empty_interpolated_string_value => InterpolatedStringExpression::empty(),
        interpolated_string_with_true_value => InterpolatedStringExpression::empty()
            .with_segment(Expression::from(true)),
        identifier => Expression::identifier("foo"),
        identifier_in_parentheses => Expression::identifier("foo").in_parentheses(),
        binary_false_and_call => BinaryExpression::new(
            BinaryOperator::And,
            Expression::from(false),
            FunctionCall::from_name("foo"),
        ),
        binary_true_or_call => BinaryExpression::new(
            BinaryOperator::Or,
            Expression::from(true),
            FunctionCall::from_name("foo"),
        ),
        not_variable => UnaryExpression::new(UnaryOperator::Not, Identifier::new("var")),
    );

    mod assume_pure_metamethods {
        use super::*;

        macro_rules! has_no_side_effects {
            ($($name:ident => $expression:expr),* $(,)?) => {
                $(
                    #[test]
                    fn $name() {
                        let evaluator = Evaluator::default().assume_pure_metamethods();
                        assert!(!evaluator.has_side_effects(&$expression.into()));
                    }
                )*
            };
        }

        has_no_side_effects!(
            addition_unknown_variable_and_number => BinaryExpression::new(
                BinaryOperator::Plus,
                Expression::identifier("foo"),
                1.0,
            ),
            addition_number_with_unknown_variable => BinaryExpression::new(
                BinaryOperator::Plus,
                1.0,
                Expression::identifier("foo"),
            ),
            unary_minus_on_variable => UnaryExpression::new(UnaryOperator::Minus, Identifier::new("var")),
            length_on_variable => UnaryExpression::new(UnaryOperator::Length, Identifier::new("var")),
            not_on_variable => UnaryExpression::new(UnaryOperator::Not, Identifier::new("var")),
            field_index => FieldExpression::new(Identifier::new("var"), "field"),
        );
    }

    #[test]
    fn evaluates_arithmetic_constants() {
        use crate::nodes::LastStatement;

        let block = crate::Parser::default()
            .parse("return 10 * 5 + 2")
            .expect("the source must parse");
        let Some(LastStatement::Return(statement)) = block.get_last_statement() else {
            panic!("expected a return statement");
        };
        let expression = statement
            .iter_expressions()
            .next()
            .expect("the return has an expression");

        let value = Evaluator::default().evaluate(expression);
        assert_eq!(value, LuaValue::Number(52.0));
        assert_eq!(value.is_truthy(), Some(true));
    }
}
