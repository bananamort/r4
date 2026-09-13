//! Conservative escape and locality analysis for local values.
//!
//! [`EscapeAnalysis`] answers whether a local symbol can be observed outside
//! the expressions that only read its fields: whether it is ever passed to a
//! function, returned, stored elsewhere, compared, indexed dynamically or
//! captured. A local table that never escapes is *confined*, and the stores to
//! its never-read fields can be removed soundly (see
//! [`remove_dead_table_stores`](crate::rules::canonicalize::remove_dead_table_stores)).
//!
//! The analysis over-approximates escape: anything it does not understand marks
//! the symbol as escaping. It never claims a table is confined unless every
//! occurrence is a static field access, so it can only miss optimizations,
//! never remove a live store.

use std::collections::{HashMap, HashSet};

use crate::nodes::{
    Arguments, AssignStatement, Block, CompoundAssignStatement, Expression, ExpressionType,
    FunctionCall, IfExpression, IfStatement, IndexExpression, InterpolatedStringExpression,
    LastStatement, NumericForStatement, Prefix, Statement, TableEntry, TableExpression, Type,
    TypeDeclarationStatement, UnaryExpression, Variable, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

use super::{SemanticModel, SymbolId};

/// What the escape analysis learned about one local symbol.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymbolEscape {
    /// Whether the symbol is initialized from a table constructor.
    pub is_table: bool,
    /// Whether the symbol may be observed as a value by other code.
    ///
    /// This includes plain local copies (`local b = a`), which are recorded in
    /// [`Self::local_copies`]; consumers that can reason about copies (such as
    /// the static linker) should use [`Self::hard_escapes`] instead.
    pub escapes: bool,
    /// Number of occurrences that genuinely expose the symbol: arguments,
    /// returns, comparisons, call targets and method calls.
    pub hard_escapes: usize,
    /// Number of plain local copies (`local b = a`).
    pub local_copies: usize,
    /// Whether the symbol is read through a non-static index (`t[expr]`).
    pub has_dynamic_read: bool,
    /// Whether the symbol is written through a non-static index (`t[expr] = v`).
    pub has_dynamic_write: bool,
    /// The statically known field names read from the symbol.
    pub field_reads: HashSet<String>,
    /// The statically known field names written on the symbol.
    pub field_writes: HashSet<String>,
}

/// The result of analyzing a block.
#[derive(Clone, Debug, Default)]
pub struct EscapeAnalysis {
    symbols: HashMap<SymbolId, SymbolEscape>,
}

impl EscapeAnalysis {
    /// Analyzes `block` against `model`.
    pub fn analyze(block: &Block, model: &SemanticModel) -> Self {
        let mut analyzer = EscapeAnalyzer {
            model,
            symbols: HashMap::new(),
        };
        analyzer.analyze_block(block);
        Self {
            symbols: analyzer.symbols,
        }
    }

    /// Returns the information collected for `symbol`, if any occurrence was
    /// seen.
    #[inline]
    pub fn info(&self, symbol: SymbolId) -> Option<&SymbolEscape> {
        self.symbols.get(&symbol)
    }

    /// Returns whether the symbol is a local table that never escapes and is
    /// never captured or reassigned.
    pub fn is_confined_table(&self, model: &SemanticModel, symbol: SymbolId) -> bool {
        let Some(info) = self.symbols.get(&symbol) else {
            return false;
        };
        info.is_table
            && !info.escapes
            && !model.symbol(symbol).is_captured()
            && model.is_immutable(symbol)
    }

    /// Returns whether the field `name` of `symbol` is read as a value.
    pub fn is_field_read(&self, symbol: SymbolId, name: &str) -> bool {
        self.symbols
            .get(&symbol)
            .map(|info| info.field_reads.contains(name))
            .unwrap_or(false)
    }

    /// Returns whether `symbol` is read through a dynamic index.
    pub fn has_dynamic_read(&self, symbol: SymbolId) -> bool {
        self.symbols
            .get(&symbol)
            .map(|info| info.has_dynamic_read)
            .unwrap_or(false)
    }

    /// Returns whether `symbol` is written through a dynamic index.
    pub fn has_dynamic_write(&self, symbol: SymbolId) -> bool {
        self.symbols
            .get(&symbol)
            .map(|info| info.has_dynamic_write)
            .unwrap_or(false)
    }

    /// Returns whether `symbol` may escape.
    pub fn escapes(&self, symbol: SymbolId) -> bool {
        self.symbols
            .get(&symbol)
            .map(|info| info.escapes)
            .unwrap_or(false)
    }

    /// Returns whether `symbol` is initialized from a table constructor.
    pub fn is_table(&self, symbol: SymbolId) -> bool {
        self.symbols
            .get(&symbol)
            .map(|info| info.is_table)
            .unwrap_or(false)
    }
}

struct EscapeAnalyzer<'m> {
    model: &'m SemanticModel,
    symbols: HashMap<SymbolId, SymbolEscape>,
}

impl EscapeAnalyzer<'_> {
    fn entry(&mut self, symbol: SymbolId) -> &mut SymbolEscape {
        self.symbols.entry(symbol).or_default()
    }

    fn mark_escape(&mut self, symbol: SymbolId) {
        let entry = self.entry(symbol);
        entry.escapes = true;
        entry.hard_escapes += 1;
    }

    fn mark_local_copy(&mut self, symbol: SymbolId) {
        let entry = self.entry(symbol);
        entry.escapes = true;
        entry.local_copies += 1;
    }

    fn symbol(&self, identifier: &crate::nodes::Identifier) -> Option<SymbolId> {
        self.model.resolve_identifier(identifier)
    }

    fn analyze_block(&mut self, block: &Block) {
        for statement in block.iter_statements() {
            self.analyze_statement(statement);
        }
        if let Some(last_statement) = block.get_last_statement() {
            self.analyze_last_statement(last_statement);
        }
    }

    fn analyze_last_statement(&mut self, statement: &LastStatement) {
        if let LastStatement::Return(return_statement) = statement {
            for expression in return_statement.iter_expressions() {
                self.analyze_value(expression);
            }
        }
    }

    fn analyze_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Assign(assignment) => self.analyze_assign(assignment),
            Statement::Call(call) => self.analyze_call(call),
            Statement::CompoundAssign(assignment) => self.analyze_compound_assign(assignment),
            Statement::Do(do_statement) => self.analyze_block(do_statement.get_block()),
            Statement::Function(function) => self.analyze_block(function.get_block()),
            Statement::GenericFor(generic_for) => {
                for expression in generic_for.iter_expressions() {
                    self.analyze_value(expression);
                }
                self.analyze_block(generic_for.get_block());
            }
            Statement::If(if_statement) => self.analyze_if_statement(if_statement),
            Statement::LocalAssign(assignment) => self.analyze_local_assign(assignment),
            Statement::LocalFunction(function) => self.analyze_block(function.get_block()),
            Statement::NumericFor(numeric_for) => self.analyze_numeric_for(numeric_for),
            Statement::Repeat(repeat) => {
                self.analyze_block(repeat.get_block());
                self.analyze_value(repeat.get_condition());
            }
            Statement::While(while_statement) => {
                self.analyze_value(while_statement.get_condition());
                self.analyze_block(while_statement.get_block());
            }
            Statement::TypeDeclaration(declaration) => self.analyze_type_declaration(declaration),
            Statement::TypeFunction(function) => self.analyze_block(function.get_block()),
            Statement::Goto(_) | Statement::Label(_) => {}
        }
    }

    fn analyze_if_statement(&mut self, if_statement: &IfStatement) {
        for branch in if_statement.iter_branches() {
            self.analyze_value(branch.get_condition());
            self.analyze_block(branch.get_block());
        }
        if let Some(else_block) = if_statement.get_else_block() {
            self.analyze_block(else_block);
        }
    }

    fn analyze_numeric_for(&mut self, numeric_for: &NumericForStatement) {
        self.analyze_value(numeric_for.get_start());
        self.analyze_value(numeric_for.get_end());
        if let Some(step) = numeric_for.get_step() {
            self.analyze_value(step);
        }
        self.analyze_block(numeric_for.get_block());
    }

    fn analyze_local_assign(&mut self, assignment: &VariableAssignment) {
        if assignment.variables_len() == assignment.values_len() {
            let variables: Vec<_> = assignment.iter_variables().collect();
            let values: Vec<_> = assignment.iter_values().collect();
            for (variable, value) in variables.iter().zip(values.iter()) {
                if matches!(value, Expression::Table(_)) {
                    if let Some(symbol) = self.symbol(variable.get_identifier()) {
                        self.entry(symbol).is_table = true;
                    }
                }
            }
        }

        for value in assignment.iter_values() {
            if let Expression::Identifier(identifier) = value {
                if let Some(symbol) = self.symbol(identifier) {
                    self.mark_local_copy(symbol);
                    continue;
                }
            }
            self.analyze_value(value);
        }
    }

    fn analyze_assign(&mut self, assignment: &AssignStatement) {
        for variable in assignment.iter_variables() {
            self.analyze_write_variable(variable);
        }

        for value in assignment.iter_values() {
            self.analyze_value(value);
        }
    }

    fn analyze_compound_assign(&mut self, assignment: &CompoundAssignStatement) {
        self.analyze_read_variable(assignment.get_variable());
        self.analyze_value(assignment.get_value());
    }

    /// Classifies a variable used as an assignment target.
    fn analyze_write_variable(&mut self, variable: &Variable) {
        match variable {
            Variable::Identifier(_) => {}
            Variable::Field(field) => {
                self.record_field_access(field.get_prefix(), field.get_field(), true)
            }
            Variable::Index(index) => {
                self.analyze_prefix_base(index.get_prefix());
                self.record_index_write(index);
                self.analyze_value(index.get_index());
            }
        }
    }

    /// Classifies a variable used both as a read and a write (`t.x += 1`).
    fn analyze_read_variable(&mut self, variable: &Variable) {
        match variable {
            Variable::Identifier(identifier) => {
                if let Some(symbol) = self.symbol(identifier) {
                    self.mark_escape(symbol);
                }
            }
            Variable::Field(field) => {
                self.record_field_access(field.get_prefix(), field.get_field(), true)
            }
            Variable::Index(index) => {
                self.analyze_prefix_base(index.get_prefix());
                self.record_index_read(index);
                self.record_index_write(index);
                self.analyze_value(index.get_index());
            }
        }
    }

    fn record_field_access(
        &mut self,
        prefix: &Prefix,
        field: &crate::nodes::Identifier,
        write: bool,
    ) {
        self.analyze_prefix_base(prefix);
        if let Prefix::Identifier(identifier) = prefix {
            if let Some(symbol) = self.symbol(identifier) {
                let name = field.get_name().clone();
                if write {
                    self.entry(symbol).field_writes.insert(name);
                } else {
                    self.entry(symbol).field_reads.insert(name);
                }
            }
        }
    }

    fn record_index_access(&mut self, index: &IndexExpression) {
        if let Prefix::Identifier(identifier) = index.get_prefix() {
            if let Some(symbol) = self.symbol(identifier) {
                match string_literal(index.get_index()) {
                    Some(name) => {
                        self.entry(symbol).field_reads.insert(name);
                    }
                    None => self.entry(symbol).has_dynamic_read = true,
                }
            }
        }
        self.analyze_value(index.get_index());
    }

    fn record_index_read(&mut self, index: &IndexExpression) {
        if let Prefix::Identifier(identifier) = index.get_prefix() {
            if let Some(symbol) = self.symbol(identifier) {
                match string_literal(index.get_index()) {
                    Some(name) => {
                        self.entry(symbol).field_reads.insert(name);
                    }
                    None => self.entry(symbol).has_dynamic_read = true,
                }
            }
        }
    }

    fn record_index_write(&mut self, index: &IndexExpression) {
        if let Prefix::Identifier(identifier) = index.get_prefix() {
            if let Some(symbol) = self.symbol(identifier) {
                match string_literal(index.get_index()) {
                    Some(name) => {
                        self.entry(symbol).field_writes.insert(name);
                    }
                    None => self.entry(symbol).has_dynamic_write = true,
                }
            }
        }
    }

    fn analyze_call(&mut self, call: &FunctionCall) {
        let prefix = call.get_prefix();
        if call.get_method().is_some() {
            self.mark_prefix_escaping(prefix);
        } else if let Some(symbol) = self.direct_identifier(prefix) {
            self.mark_escape(symbol);
        } else {
            self.analyze_prefix_base(prefix);
        }
        self.analyze_arguments(call.get_arguments());
    }

    fn mark_prefix_escaping(&mut self, prefix: &Prefix) {
        match prefix {
            Prefix::Identifier(identifier) => {
                if let Some(symbol) = self.symbol(identifier) {
                    self.mark_escape(symbol);
                }
            }
            Prefix::Field(field) => self.mark_prefix_escaping(field.get_prefix()),
            Prefix::Index(index) => {
                self.mark_prefix_escaping(index.get_prefix());
                self.analyze_value(index.get_index());
            }
            Prefix::Parenthese(parenthese) => {
                self.analyze_value(parenthese.inner_expression());
            }
            Prefix::Call(call) => self.analyze_call(call),
            Prefix::TypeInstantiation(instantiation) => {
                self.mark_prefix_escaping(instantiation.get_prefix());
            }
        }
    }

    fn direct_identifier(&self, prefix: &Prefix) -> Option<SymbolId> {
        match prefix {
            Prefix::Identifier(identifier) => self.symbol(identifier),
            Prefix::Parenthese(parenthese) => match parenthese.inner_expression() {
                Expression::Identifier(identifier) => self.symbol(identifier),
                _ => None,
            },
            _ => None,
        }
    }

    fn analyze_arguments(&mut self, arguments: &Arguments) {
        match arguments {
            Arguments::Tuple(tuple) => {
                for value in tuple.iter_values() {
                    self.analyze_value(value);
                }
            }
            Arguments::String(_) => {}
            Arguments::Table(table) => self.analyze_table(table),
        }
    }

    /// Analyzes a prefix used as the base of a field or index access.
    fn analyze_prefix_base(&mut self, prefix: &Prefix) {
        match prefix {
            Prefix::Identifier(_) => {}
            Prefix::Field(field) => {
                self.record_field_access(field.get_prefix(), field.get_field(), false)
            }
            Prefix::Index(index) => {
                self.analyze_prefix_base(index.get_prefix());
                self.record_index_access(index)
            }
            Prefix::Parenthese(parenthese) => {
                self.analyze_base(parenthese.inner_expression());
            }
            Prefix::Call(call) => self.analyze_call(call),
            Prefix::TypeInstantiation(instantiation) => {
                self.mark_prefix_escaping(instantiation.get_prefix());
            }
        }
    }

    /// Analyzes an expression used as the base of a field or index access.
    fn analyze_base(&mut self, expression: &Expression) {
        match expression {
            Expression::Identifier(_) => {}
            Expression::Field(field) => {
                self.record_field_access(field.get_prefix(), field.get_field(), false)
            }
            Expression::Index(index) => {
                self.analyze_prefix_base(index.get_prefix());
                self.record_index_access(index)
            }
            Expression::Parenthese(parenthese) => self.analyze_base(parenthese.inner_expression()),
            Expression::Call(call) => self.analyze_call(call),
            other => self.analyze_value(other),
        }
    }

    /// Analyzes an expression whose value is consumed.
    fn analyze_value(&mut self, expression: &Expression) {
        match expression {
            Expression::Binary(binary) => {
                self.analyze_value(binary.left());
                self.analyze_value(binary.right());
            }
            Expression::Call(call) => self.analyze_call(call),
            Expression::Field(field) => {
                self.record_field_access(field.get_prefix(), field.get_field(), false)
            }
            Expression::Function(function) => self.analyze_block(function.get_block()),
            Expression::Identifier(identifier) => {
                if let Some(symbol) = self.symbol(identifier) {
                    self.mark_escape(symbol);
                }
            }
            Expression::If(if_expression) => self.analyze_if_expression(if_expression),
            Expression::Index(index) => {
                self.analyze_prefix_base(index.get_prefix());
                self.record_index_access(index);
            }
            Expression::InterpolatedString(string) => self.analyze_interpolated_string(string),
            Expression::Parenthese(parenthese) => self.analyze_value(parenthese.inner_expression()),
            Expression::Table(table) => self.analyze_table(table),
            Expression::Unary(unary) => self.analyze_unary(unary),
            Expression::TypeCast(type_cast) => self.analyze_value(type_cast.get_expression()),
            Expression::TypeInstantiation(instantiation) => {
                self.mark_prefix_escaping(instantiation.get_prefix());
            }
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::String(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => {}
        }
    }

    fn analyze_if_expression(&mut self, if_expression: &IfExpression) {
        self.analyze_value(if_expression.get_condition());
        self.analyze_value(if_expression.get_result());
        for branch in if_expression.iter_branches() {
            self.analyze_value(branch.get_condition());
            self.analyze_value(branch.get_result());
        }
        self.analyze_value(if_expression.get_else_result());
    }

    fn analyze_unary(&mut self, unary: &UnaryExpression) {
        self.analyze_value(unary.get_expression());
    }

    fn analyze_interpolated_string(&mut self, string: &InterpolatedStringExpression) {
        for segment in string.iter_segments() {
            if let crate::nodes::InterpolationSegment::Value(value) = segment {
                self.analyze_value(value.get_expression());
            }
        }
    }

    fn analyze_table(&mut self, table: &TableExpression) {
        for entry in table.iter_entries() {
            match entry {
                TableEntry::Field(field) => self.analyze_value(field.get_value()),
                TableEntry::Index(index) => {
                    self.analyze_value(index.get_key());
                    self.analyze_value(index.get_value());
                }
                TableEntry::Value(value) => self.analyze_value(value),
            }
        }
    }

    fn analyze_type_declaration(&mut self, declaration: &TypeDeclarationStatement) {
        self.analyze_type(declaration.get_type());
    }

    fn analyze_type(&mut self, r#type: &Type) {
        let mut r#type = r#type.clone();
        let mut processor = TypeExpressionProcessor { analyzer: self };
        DefaultVisitor::visit_type(&mut r#type, &mut processor);
    }
}

struct TypeExpressionProcessor<'a, 'm> {
    analyzer: &'a mut EscapeAnalyzer<'m>,
}

impl NodeProcessor for TypeExpressionProcessor<'_, '_> {
    fn process_expression_type(&mut self, expression_type: &mut ExpressionType) {
        self.analyzer
            .analyze_value(expression_type.get_expression());
    }
}

fn string_literal(expression: &Expression) -> Option<String> {
    match expression {
        Expression::String(string) => std::str::from_utf8(string.get_value())
            .ok()
            .map(str::to_owned),
        _ => None,
    }
}
