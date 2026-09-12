mod lua_value;

pub use lua_value::*;

use crate::nodes::*;

/// A struct to convert an Expression node into a LuaValue object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evaluator {
    pure_metamethods: bool,
}

impl Evaluator {
    /// When evaluating expressions related to tables, this value tells the evaluator if
    /// metamethods can have side effects. For example, indexing a normal table in Lua does not
    /// have any side effects, but if the table is a metatable, it's `__index` metamethod can
    /// possibly have side effects (since it can be a function call).
    pub fn assume_pure_metamethods(mut self) -> Self {
        self.pure_metamethods = true;
        self
    }

    pub fn evaluate(&self, expression: &Expression) -> LuaValue {
        match expression {
            Expression::False(_) => LuaValue::False,
            Expression::Function(_) => LuaValue::Function,
            Expression::Nil(_) => LuaValue::Nil,
            Expression::Number(number) => LuaValue::from(number.compute_value()),
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

    pub fn has_side_effects(&self, expression: &Expression) -> bool {
        match expression {
            Expression::False(_)
            | Expression::Function(_)
            | Expression::Identifier(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::String(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => false,
            Expression::If(if_expression) => self.if_expression_has_side_effects(if_expression),
            Expression::Binary(binary) => {
                let left = binary.left();
                let right = binary.right();

                let left_value = self.evaluate(left);
                let left_side_effect = self.has_side_effects(binary.left());

                match binary.operator() {
                    BinaryOperator::And => {
                        if left_value.is_truthy().unwrap_or(true) {
                            left_side_effect || self.has_side_effects(binary.right())
                        } else {
                            left_side_effect
                        }
                    }
                    BinaryOperator::Or => {
                        if left_value.is_truthy().unwrap_or(false) {
                            left_side_effect
                        } else {
                            left_side_effect || self.has_side_effects(binary.right())
                        }
                    }
                    _ => {
                        if self.pure_metamethods {
                            left_side_effect || self.has_side_effects(binary.right())
                        } else {
                            self.maybe_metatable(&left_value)
                                || self.maybe_metatable(&self.evaluate(right))
                                || self.has_side_effects(left)
                                || self.has_side_effects(right)
                        }
                    }
                }
            }
            Expression::Unary(unary) => {
                if self.pure_metamethods || matches!(unary.operator(), UnaryOperator::Not) {
                    self.has_side_effects(unary.get_expression())
                } else {
                    let sub_expression = unary.get_expression();

                    self.maybe_metatable(&self.evaluate(sub_expression))
                        || self.has_side_effects(sub_expression)
                }
            }
            Expression::Field(field) => self.field_has_side_effects(field),
            Expression::Index(index) => self.index_has_side_effects(index),
            Expression::Parenthese(parenthese) => {
                self.has_side_effects(parenthese.inner_expression())
            }
            Expression::Table(table) => table
                .get_entries()
                .iter()
                .any(|entry| self.table_entry_has_side_effects(entry)),
            Expression::Call(call) => self.call_has_side_effects(call),
            Expression::InterpolatedString(interpolated_string) => interpolated_string
                .iter_segments()
                .any(|segment| match segment {
                    InterpolationSegment::String(_) => false,
                    InterpolationSegment::Value(value) => {
                        self.has_side_effects(value.get_expression())
                    }
                }),
            Expression::TypeCast(type_cast) => self.has_side_effects(type_cast.get_expression()),
            Expression::TypeInstantiation(type_instantiation) => {
                self.type_instantiation_has_side_effects(type_instantiation)
            }
        }
    }

    fn if_expression_has_side_effects(&self, if_expression: &IfExpression) -> bool {
        if self.has_side_effects(if_expression.get_condition()) {
            return true;
        }

        let condition = self.evaluate(if_expression.get_condition());

        if let Some(truthy) = condition.is_truthy() {
            if truthy {
                self.has_side_effects(if_expression.get_result())
            } else {
                for branch in if_expression.iter_branches() {
                    if self.has_side_effects(branch.get_condition()) {
                        return true;
                    }

                    let branch_condition = self.evaluate(branch.get_condition());

                    if let Some(truthy) = branch_condition.is_truthy() {
                        if truthy {
                            return self.has_side_effects(branch.get_result());
                        }
                    } else if self.has_side_effects(branch.get_result()) {
                        return true;
                    }
                }

                self.has_side_effects(if_expression.get_else_result())
            }
        } else {
            if self.has_side_effects(if_expression.get_result()) {
                return true;
            }

            for branch in if_expression.iter_branches() {
                if self.has_side_effects(branch.get_condition())
                    || self.has_side_effects(branch.get_result())
                {
                    return true;
                }
            }

            self.has_side_effects(if_expression.get_else_result())
        }
    }

    #[inline]
    fn call_has_side_effects(&self, _call: &FunctionCall) -> bool {
        true
    }

    #[inline]
    fn table_entry_has_side_effects(&self, entry: &TableEntry) -> bool {
        match entry {
            TableEntry::Field(entry) => self.has_side_effects(entry.get_value()),
            TableEntry::Index(entry) => {
                self.has_side_effects(entry.get_key()) || self.has_side_effects(entry.get_value())
            }
            TableEntry::Value(value) => self.has_side_effects(value),
        }
    }

    #[inline]
    fn field_has_side_effects(&self, field: &FieldExpression) -> bool {
        !self.pure_metamethods || self.prefix_has_side_effects(field.get_prefix())
    }

    #[inline]
    fn index_has_side_effects(&self, index: &IndexExpression) -> bool {
        !self.pure_metamethods
            || self.has_side_effects(index.get_index())
            || self.prefix_has_side_effects(index.get_prefix())
    }

    fn prefix_has_side_effects(&self, prefix: &Prefix) -> bool {
        match prefix {
            Prefix::Call(call) => self.call_has_side_effects(call),
            Prefix::Field(field) => self.field_has_side_effects(field),
            Prefix::Identifier(_) => false,
            Prefix::Index(index) => self.index_has_side_effects(index),
            Prefix::Parenthese(sub_expression) => {
                self.has_side_effects(sub_expression.inner_expression())
            }
            Prefix::TypeInstantiation(type_instantiation) => {
                self.type_instantiation_has_side_effects(type_instantiation)
            }
        }
    }

    fn type_instantiation_has_side_effects(
        &self,
        type_instantiation: &TypeInstantiationExpression,
    ) -> bool {
        self.prefix_has_side_effects(type_instantiation.get_prefix())
    }

    #[inline]
    fn maybe_metatable(&self, value: &LuaValue) -> bool {
        match value {
            LuaValue::False
            | LuaValue::Function
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
            BinaryOperator::Plus => self.evaluate_math(expression, |a, b| a + b),
            BinaryOperator::Minus => self.evaluate_math(expression, |a, b| a - b),
            BinaryOperator::Asterisk => self.evaluate_math(expression, |a, b| a * b),
            BinaryOperator::Slash => self.evaluate_math(expression, |a, b| a / b),
            BinaryOperator::DoubleSlash => self.evaluate_math(expression, |a, b| (a / b).floor()),
            BinaryOperator::Caret => self.evaluate_math(expression, |a, b| a.powf(b)),
            BinaryOperator::Percent => {
                self.evaluate_math(expression, |a, b| a - b * (a / b).floor())
            }
            BinaryOperator::Concat => {
                match (
                    self.evaluate(expression.left()).string_coercion(),
                    self.evaluate(expression.right()).string_coercion(),
                ) {
                    (LuaValue::String(mut left), LuaValue::String(right)) => {
                        left.extend_from_slice(&right);
                        LuaValue::String(left)
                    }
                    _ => LuaValue::Unknown,
                }
            }
            BinaryOperator::LowerThan => self.evaluate_relational(expression, |a, b| a < b),
            BinaryOperator::LowerOrEqualThan => self.evaluate_relational(expression, |a, b| a <= b),
            BinaryOperator::GreaterThan => self.evaluate_relational(expression, |a, b| a > b),
            BinaryOperator::GreaterOrEqualThan => {
                self.evaluate_relational(expression, |a, b| a >= b)
            }
        }
    }

    fn evaluate_equal(&self, left: &LuaValue, right: &LuaValue) -> LuaValue {
        match (left, right) {
            (LuaValue::Unknown, _) | (_, LuaValue::Unknown) => LuaValue::Unknown,
            (LuaValue::True, LuaValue::True)
            | (LuaValue::False, LuaValue::False)
            | (LuaValue::Nil, LuaValue::Nil) => LuaValue::True,
            (LuaValue::Number(a), LuaValue::Number(b)) => {
                LuaValue::from((a - b).abs() < f64::EPSILON)
            }
            (LuaValue::String(a), LuaValue::String(b)) => LuaValue::from(a == b),
            _ => LuaValue::False,
        }
    }

    fn evaluate_math<F>(&self, expression: &BinaryExpression, operation: F) -> LuaValue
    where
        F: Fn(f64, f64) -> f64,
    {
        let left = self.evaluate(expression.left()).number_coercion();

        if let LuaValue::Number(left) = left {
            let right = self.evaluate(expression.right()).number_coercion();

            if let LuaValue::Number(right) = right {
                LuaValue::Number(operation(left, right))
            } else {
                LuaValue::Unknown
            }
        } else {
            LuaValue::Unknown
        }
    }

    fn evaluate_relational<F>(&self, expression: &BinaryExpression, operation: F) -> LuaValue
    where
        F: Fn(f64, f64) -> bool,
    {
        let left = self.evaluate(expression.left());

        match left {
            LuaValue::Number(left) => {
                let right = self.evaluate(expression.right());

                if let LuaValue::Number(right) = right {
                    if operation(left, right) {
                        LuaValue::True
                    } else {
                        LuaValue::False
                    }
                } else {
                    LuaValue::Unknown
                }
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
                    LuaValue::Number(value) => LuaValue::from(-value),
                    _ => LuaValue::Unknown,
                }
            }
            UnaryOperator::Length => self.evaluate(expression.get_expression()).length(),
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
            Expression::Number(number) => Some(LuaValue::from(number.compute_value())),
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
                Self::concatenate(left, right)
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
                    _ => Self::evaluate_math_operator(operator, left, right),
                }
            }
        }
    }

    fn evaluate_math_operator(
        operator: BinaryOperator,
        left: LuaValue,
        right: LuaValue,
    ) -> Option<LuaValue> {
        let LuaValue::Number(left) = left.number_coercion() else {
            return None;
        };
        let LuaValue::Number(right) = right.number_coercion() else {
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

    fn evaluate_relational_operator(
        &self,
        operator: BinaryOperator,
        left: LuaValue,
        right: LuaValue,
    ) -> Option<LuaValue> {
        match (left, right) {
            (LuaValue::Number(left), LuaValue::Number(right)) => {
                let result = match operator {
                    BinaryOperator::LowerThan => left < right,
                    BinaryOperator::LowerOrEqualThan => left <= right,
                    BinaryOperator::GreaterThan => left > right,
                    BinaryOperator::GreaterOrEqualThan => left >= right,
                    _ => return None,
                };
                Some(LuaValue::from(result))
            }
            (LuaValue::String(left), LuaValue::String(right)) => {
                match self.compare_strings(&left, &right, operator) {
                    LuaValue::Unknown => None,
                    value => Some(value),
                }
            }
            _ => None,
        }
    }

    fn concatenate(left: LuaValue, right: LuaValue) -> Option<LuaValue> {
        match (left.string_coercion(), right.string_coercion()) {
            (LuaValue::String(mut left), LuaValue::String(right)) => {
                left.extend_from_slice(&right);
                Some(LuaValue::String(left))
            }
            _ => None,
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
                LuaValue::Number(number) => Some(LuaValue::from(-number)),
                _ => None,
            },
            UnaryOperator::Length => match value.length() {
                LuaValue::Unknown => None,
                length => Some(length),
            },
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
}
