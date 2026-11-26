//! Inlining of calls to local functions.
//!
//! [`inline_local_functions`] replaces a call to a local function with the
//! function's body bound to the arguments at the call site. The function is left
//! in place; [`remove_unused_variables`](super::remove_unused_variables) removes
//! it once its last call has gone.
//!
//! A call is rewritten into a statement-level preamble that runs where the call
//! did, and the call expression becomes the result variables:
//!
//! ```lua
//! local __r1, __r2                            -- the call's results
//! do
//!     local p1, p2, __d1 = <arg 1>, <arg 2>, <extra arg>
//!     local __done = false
//!     repeat
//!         <body, with `return e1, e2` lowered to `__r1, __r2 = e1, e2; __done = true; break`
//!          and `if __done then break end` after every loop>
//!     until true
//! end
//! ```
//!
//! The arguments bind to the parameters exactly as the call did, so each is
//! evaluated once, in order, with the same multi-value adjustment; extra
//! arguments are evaluated into fresh unused locals. The parameters keep their
//! names inside the `do`, so the body's references resolve to the arguments
//! exactly as they resolved to the parameters, and the body's own locals stay
//! scoped to the inline.
//!
//! The `repeat ... until true` runs the body once and gives every `return` a
//! `break` to leave through; the `if __done then break end` after each loop
//! carries the exit out of any loop the `return` sits in, and the `__done` flag
//! keeps a loop's own `break` from being mistaken for it.
//!
//! When the call's values are consumed (the last position of a value list,
//! where Lua takes every value), the replacement is `__r1, ..., __rk`; in a
//! single-value position it is `__r1` (or `nil` when the body yields none). A
//! body that can fall through yields no values, so its arity is zero.
//!
//! Conditions that make an inline impossible are not worked around:
//!
//! * a body that reads `...` cannot be reproduced, because `...` is lexical to
//!   a function and no expression denotes a foreign vararg list;
//! * a call is left alone when its callee cannot be resolved to an immutable,
//!   uncaptured local function.
//!
//! Everything else — statements, `if`, loops, `return` inside loops,
//! fall-through, any number of results, upvalue reads, and `goto`/labels — is
//! handled.

use std::collections::HashMap;

use crate::nodes::{
    AssignStatement, Block, DoStatement, Expression, FunctionAssignment, Identifier, LastStatement,
    Prefix, RepeatStatement, Statement, TypedIdentifier, Variable, VariableAssignment,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::{LabelId, ScopeId, SemanticModel, SymbolId, SymbolKind};

/// A local function that can be inlined at its call sites.
struct InlinableFunction {
    /// The parameter names, in declaration order.
    parameters: Vec<String>,
    /// The body, which reads its parameters, its own locals, globals, and the
    /// upvalues in [`Self::upvalues`].
    body: Block,
    /// The number of values every `return` yields. Zero means the call yields
    /// nothing, either because every `return` is bare or because the body can
    /// fall through.
    arity: usize,
    /// The symbols the body reads from outside the function.
    upvalues: Vec<SymbolId>,
}

/// Replaces calls to inlinable local functions by their body.
///
/// Returns the number of calls inlined.
pub fn inline_local_functions(block: &mut Block, model: &SemanticModel) -> usize {
    let mut collector = FunctionCollector {
        model,
        functions: HashMap::new(),
    };
    DefaultVisitor::visit_block(block, &mut collector);

    if collector.functions.is_empty() {
        return 0;
    }

    let mut names = NameGenerator::new(model);
    let mut inlined = 0;
    let mut inliner = Inliner {
        model,
        functions: &collector.functions,
        names: &mut names,
        current_scope: None,
        inlined: &mut inlined,
    };
    inliner.rewrite_block(block);

    inlined
}

// ----------------------------------------------------------------------
// Deciding what can be inlined
// ----------------------------------------------------------------------

fn inlinable(
    model: &SemanticModel,
    symbol: SymbolId,
    body: &Block,
    parameters: &[&TypedIdentifier],
) -> Option<InlinableFunction> {
    let info = model.symbol(symbol);
    if info.kind != SymbolKind::Local || !model.is_immutable(symbol) {
        return None;
    }

    let mut check = BodyCheck {
        model,
        scope: model.try_scope_of(body).ok()?,
        loop_depth: 0,
        function_depth: 0,
        arity: None,
        upvalues: Vec::new(),
        rejects: false,
    };
    check.visit(body);

    if check.rejects {
        return None;
    }

    let arity = check.arity.unwrap_or(0);
    // Several results require every path to return them; a body that can fall
    // through yields nothing on that path.
    if arity > 0 && !matches!(body.get_last_statement(), Some(LastStatement::Return(_))) {
        return None;
    }

    Some(InlinableFunction {
        parameters: parameters
            .iter()
            .map(|parameter| parameter.get_identifier().get_name().clone())
            .collect(),
        body: body.clone(),
        arity,
        upvalues: check.upvalues,
    })
}

/// Measures a body and collects what it reads from outside the function.
struct BodyCheck<'m> {
    model: &'m SemanticModel,
    /// The function's own scope; a symbol declared outside it is an upvalue.
    scope: ScopeId,
    loop_depth: usize,
    /// Zero while inside the function whose calls are being inlined.
    function_depth: usize,
    arity: Option<usize>,
    upvalues: Vec<SymbolId>,
    rejects: bool,
}

impl BodyCheck<'_> {
    fn visit(&mut self, block: &Block) {
        for statement in block.iter_statements() {
            self.visit_statement(statement);
        }
        if let Some(last) = block.get_last_statement() {
            self.visit_last_statement(last);
        }
    }

    fn visit_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Assign(assignment) => {
                for variable in assignment.iter_variables() {
                    self.visit_variable(variable);
                }
                for value in assignment.iter_values() {
                    self.visit_expression(value);
                }
            }
            Statement::LocalAssign(assignment) => {
                for value in assignment.iter_values() {
                    self.visit_expression(value);
                }
            }
            Statement::CompoundAssign(assignment) => {
                self.visit_variable(assignment.get_variable());
                self.visit_expression(assignment.get_value());
            }
            Statement::Call(call) => {
                for value in call.get_arguments().clone().to_expressions() {
                    self.visit_expression(&value);
                }
                self.visit_prefix(call.get_prefix());
            }
            Statement::While(statement) => {
                self.visit_expression(statement.get_condition());
                self.visit_loop(statement.get_block());
            }
            Statement::Repeat(statement) => {
                self.visit_loop(statement.get_block());
                self.visit_expression(statement.get_condition());
            }
            Statement::NumericFor(statement) => {
                self.visit_expression(statement.get_start());
                self.visit_expression(statement.get_end());
                if let Some(step) = statement.get_step() {
                    self.visit_expression(step);
                }
                self.visit_loop(statement.get_block());
            }
            Statement::GenericFor(statement) => {
                for value in statement.iter_expressions() {
                    self.visit_expression(value);
                }
                self.visit_loop(statement.get_block());
            }
            Statement::Do(statement) => self.visit(statement.get_block()),
            Statement::Function(statement) => self.visit_function(statement.get_block()),
            Statement::LocalFunction(statement) => self.visit_function(statement.get_block()),
            Statement::TypeFunction(statement) => self.visit_function(statement.get_block()),
            Statement::If(statement) => {
                for branch in statement.iter_branches() {
                    self.visit_expression(branch.get_condition());
                    self.visit(branch.get_block());
                }
                if let Some(else_block) = statement.get_else_block() {
                    self.visit(else_block);
                }
            }
            _ => {}
        }
    }

    fn visit_variable(&mut self, variable: &Variable) {
        match variable {
            Variable::Identifier(_) => {}
            Variable::Field(field) => self.visit_prefix(field.get_prefix()),
            Variable::Index(index) => {
                self.visit_prefix(index.get_prefix());
                self.visit_expression(index.get_index());
            }
        }
    }

    fn visit_loop(&mut self, block: &Block) {
        self.loop_depth += 1;
        self.visit(block);
        self.loop_depth -= 1;
    }

    fn visit_function(&mut self, block: &Block) {
        self.function_depth += 1;
        self.visit(block);
        self.function_depth -= 1;
    }

    fn visit_last_statement(&mut self, statement: &LastStatement) {
        let LastStatement::Return(return_statement) = statement else {
            return;
        };
        if self.function_depth != 0 {
            return;
        }

        let expressions: Vec<&Expression> = return_statement.iter_expressions().collect();
        for expression in &expressions {
            if matches!(
                expression,
                Expression::Call(_) | Expression::VariableArguments(_)
            ) {
                // The return yields an unknown number of values.
                self.rejects = true;
                return;
            }
            self.visit_expression(expression);
        }

        match self.arity {
            None => self.arity = Some(expressions.len()),
            Some(arity) if arity == expressions.len() => {}
            Some(_) => self.rejects = true,
        }
    }

    fn visit_expression(&mut self, expression: &Expression) {
        if self.rejects {
            return;
        }
        match expression {
            Expression::VariableArguments(_) => {
                if self.function_depth == 0 {
                    // `...` cannot be forwarded through an inline.
                    self.rejects = true;
                }
            }
            Expression::Identifier(identifier) => {
                let Some(symbol) = self.model.resolve_identifier(identifier) else {
                    self.rejects = true;
                    return;
                };
                if self.model.symbol(symbol).kind == SymbolKind::Global {
                    return;
                }
                if !self.is_declared_in_function(symbol) && !self.upvalues.contains(&symbol) {
                    self.upvalues.push(symbol);
                }
            }
            Expression::Function(function) => self.visit_function(function.get_block()),
            Expression::Binary(binary) => {
                self.visit_expression(binary.left());
                self.visit_expression(binary.right());
            }
            Expression::Unary(unary) => self.visit_expression(unary.get_expression()),
            Expression::Parenthese(parenthese) => {
                self.visit_expression(parenthese.inner_expression())
            }
            Expression::Index(index) => {
                self.visit_expression(index.get_index());
                self.visit_prefix(index.get_prefix());
            }
            Expression::Field(field) => self.visit_prefix(field.get_prefix()),
            Expression::Call(call) => {
                for argument in call.get_arguments().clone().to_expressions() {
                    self.visit_expression(&argument);
                }
                self.visit_prefix(call.get_prefix());
            }
            Expression::If(if_expression) => {
                self.visit_expression(if_expression.get_condition());
                self.visit_expression(if_expression.get_result());
                for branch in if_expression.iter_branches() {
                    self.visit_expression(branch.get_condition());
                    self.visit_expression(branch.get_result());
                }
                self.visit_expression(if_expression.get_else_result());
            }
            Expression::Table(table) => {
                for entry in table.iter_entries() {
                    match entry {
                        crate::nodes::TableEntry::Field(field) => {
                            self.visit_expression(field.get_value())
                        }
                        crate::nodes::TableEntry::Index(index) => {
                            self.visit_expression(index.get_key());
                            self.visit_expression(index.get_value());
                        }
                        crate::nodes::TableEntry::Value(value) => self.visit_expression(value),
                    }
                }
            }
            Expression::InterpolatedString(string) => {
                for segment in string.iter_segments() {
                    if let crate::nodes::InterpolationSegment::Value(value) = segment {
                        self.visit_expression(value.get_expression());
                    }
                }
            }
            Expression::TypeCast(type_cast) => self.visit_expression(type_cast.get_expression()),
            _ => {}
        }
    }

    fn visit_prefix(&mut self, prefix: &Prefix) {
        match prefix {
            Prefix::Identifier(_) => {}
            Prefix::Field(field) => self.visit_prefix(field.get_prefix()),
            Prefix::Index(index) => {
                self.visit_prefix(index.get_prefix());
                self.visit_expression(index.get_index());
            }
            Prefix::Call(call) => {
                for argument in call.get_arguments().clone().to_expressions() {
                    self.visit_expression(&argument);
                }
                self.visit_prefix(call.get_prefix());
            }
            Prefix::Parenthese(parenthese) => self.visit_expression(parenthese.inner_expression()),
            Prefix::TypeInstantiation(instantiation) => {
                self.visit_prefix(instantiation.get_prefix())
            }
        }
    }

    fn is_declared_in_function(&self, symbol: SymbolId) -> bool {
        let mut scope = Some(self.model.symbol(symbol).declaring_scope);
        while let Some(id) = scope {
            if id == self.scope {
                return true;
            }
            scope = self.model.scope(id).parent;
        }
        false
    }
}

// ----------------------------------------------------------------------
// Collecting the functions
// ----------------------------------------------------------------------

struct FunctionCollector<'m> {
    model: &'m SemanticModel,
    functions: HashMap<SymbolId, InlinableFunction>,
}

impl FunctionCollector<'_> {
    fn record(&mut self, identifier: &Identifier, body: &Block, parameters: &[&TypedIdentifier]) {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return;
        };
        if let Some(function) = inlinable(self.model, symbol, body, parameters) {
            self.functions.insert(symbol, function);
        }
    }
}

impl NodeProcessor for FunctionCollector<'_> {
    fn process_local_function_statement(&mut self, statement: &mut FunctionAssignment) {
        let parameters: Vec<&TypedIdentifier> = statement.iter_parameters().collect();
        let (identifier, body) = (statement.get_identifier(), statement.get_block());
        self.record(identifier, body, &parameters);
    }

    fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
        if assignment.variables_len() != 1 || assignment.values_len() != 1 {
            return;
        }
        let Some(Expression::Function(function)) = assignment.iter_values().next() else {
            return;
        };
        let Some(variable) = assignment.iter_variables().next() else {
            return;
        };
        let parameters: Vec<&TypedIdentifier> = function.iter_parameters().collect();
        let identifier = variable.get_identifier();
        let body = function.get_block();
        self.record(identifier, body, &parameters);
    }
}

// ----------------------------------------------------------------------
// Rewriting
// ----------------------------------------------------------------------

/// Rewrites every block and expression, gathering a call's preamble before the
/// statement it belongs to. Nested blocks are rewritten by explicit recursion,
/// so an inserted preamble is never visited again.
struct Inliner<'m, 'n, 'i> {
    model: &'m SemanticModel,
    functions: &'m HashMap<SymbolId, InlinableFunction>,
    names: &'n mut NameGenerator,
    /// The scope of the block currently being rewritten.
    current_scope: Option<ScopeId>,
    inlined: &'i mut usize,
}

impl Inliner<'_, '_, '_> {
    fn rewrite_block(&mut self, block: &mut Block) {
        let previous = self.current_scope;
        let scope = self.model.try_scope_of(block).ok();
        self.current_scope = scope;
        let statements = block.take_statements();
        let mut rewritten = Vec::with_capacity(statements.len());
        for mut statement in statements {
            let mut preambles = Vec::new();
            self.rewrite_statement(&mut statement, &mut preambles);
            rewritten.append(&mut preambles);
            rewritten.push(statement);
        }
        block.set_statements(rewritten);

        if let Some(mut last) = block.take_last_statement() {
            let mut preambles = Vec::new();
            self.rewrite_last_statement(&mut last, &mut preambles);
            if !preambles.is_empty() {
                let mut statements = block.take_statements();
                statements.append(&mut preambles);
                block.set_statements(statements);
            }
            block.set_last_statement(last);
        }
        self.current_scope = previous;
    }

    fn rewrite_statement(&mut self, statement: &mut Statement, preambles: &mut Vec<Statement>) {
        match statement {
            Statement::Assign(assignment) => {
                let variables: Vec<Variable> = assignment.iter_variables().cloned().collect();
                for variable in &variables {
                    let mut variable = variable.clone();
                    self.rewrite_variable(&mut variable, preambles);
                }
                let mut values: Vec<Expression> = assignment.iter_values().cloned().collect();
                self.rewrite_values(&mut values, preambles);
                *assignment = AssignStatement::new(variables, values);
            }
            Statement::LocalAssign(assignment) => {
                let variables: Vec<TypedIdentifier> =
                    assignment.iter_variables().cloned().collect();
                let mut values: Vec<Expression> = assignment.iter_values().cloned().collect();
                self.rewrite_values(&mut values, preambles);
                *assignment = VariableAssignment::new(variables, values);
            }
            Statement::Call(slot) => {
                let mut call = slot.clone();
                self.rewrite_prefix(call.mutate_prefix(), preambles);
                let mut values: Vec<Expression> = call.get_arguments().clone().to_expressions();
                self.rewrite_values(&mut values, preambles);
                *call.mutate_arguments() =
                    crate::nodes::Arguments::Tuple(crate::nodes::TupleArguments::from_iter(values));
                if self.try_inline(&call, false, preambles).is_some() {
                    // The call's result is discarded; the body ran in the preamble.
                    *statement = Statement::Do(DoStatement::new(Block::new(Vec::new(), None)));
                } else {
                    *slot = call;
                }
            }
            Statement::CompoundAssign(assignment) => {
                let mut variable = assignment.get_variable().clone();
                self.rewrite_variable(&mut variable, preambles);
                *assignment.mutate_variable() = variable;
                let mut value = assignment.get_value().clone();
                self.rewrite_single(&mut value, preambles);
                *assignment.mutate_value() = value;
            }
            Statement::If(statement) => {
                for branch in statement.mutate_branches() {
                    let mut condition = branch.get_condition().clone();
                    self.rewrite_single(&mut condition, preambles);
                    *branch.mutate_condition() = condition;
                    self.rewrite_block(branch.mutate_block());
                }
                if let Some(else_block) = statement.mutate_else_block() {
                    self.rewrite_block(else_block);
                }
            }
            Statement::While(statement) => {
                let mut condition = statement.get_condition().clone();
                self.rewrite_single(&mut condition, preambles);
                statement.mutate_condition().clone_from(&condition);
                self.rewrite_block(statement.mutate_block());
            }
            Statement::Repeat(statement) => {
                self.rewrite_block(statement.mutate_block());
                let mut condition = statement.get_condition().clone();
                self.rewrite_single(&mut condition, preambles);
                statement.mutate_condition().clone_from(&condition);
            }
            Statement::NumericFor(statement) => {
                let mut start = statement.get_start().clone();
                let mut end = statement.get_end().clone();
                self.rewrite_single(&mut start, preambles);
                self.rewrite_single(&mut end, preambles);
                statement.mutate_start().clone_from(&start);
                statement.mutate_end().clone_from(&end);
                if let Some(step) = statement.get_step() {
                    let mut step = step.clone();
                    self.rewrite_single(&mut step, preambles);
                    *statement.mutate_step() = Some(step);
                }
                self.rewrite_block(statement.mutate_block());
            }
            Statement::GenericFor(statement) => {
                let mut values: Vec<Expression> = statement.iter_expressions().cloned().collect();
                self.rewrite_values(&mut values, preambles);
                for (index, value) in values.into_iter().enumerate() {
                    statement.mutate_expressions()[index].clone_from(&value);
                }
                self.rewrite_block(statement.mutate_block());
            }
            Statement::Do(statement) => self.rewrite_block(statement.mutate_block()),
            Statement::Function(statement) => self.rewrite_block(statement.mutate_block()),
            Statement::LocalFunction(statement) => self.rewrite_block(statement.mutate_block()),
            Statement::TypeFunction(statement) => self.rewrite_block(statement.mutate_block()),
            _ => {}
        }
    }

    fn rewrite_last_statement(
        &mut self,
        statement: &mut LastStatement,
        preambles: &mut Vec<Statement>,
    ) {
        let LastStatement::Return(return_statement) = statement else {
            return;
        };
        let mut values: Vec<Expression> = return_statement.iter_expressions().cloned().collect();
        self.rewrite_values(&mut values, preambles);
        *return_statement = crate::nodes::ReturnStatement::new(values);
    }

    fn rewrite_variable(&mut self, variable: &mut Variable, preambles: &mut Vec<Statement>) {
        match variable {
            Variable::Identifier(_) => {}
            Variable::Field(field) => self.rewrite_prefix(field.mutate_prefix(), preambles),
            Variable::Index(index) => {
                self.rewrite_prefix(index.mutate_prefix(), preambles);
                let mut key = index.get_index().clone();
                self.rewrite_single(&mut key, preambles);
                *index.mutate_index() = key;
            }
        }
    }

    fn rewrite_values(&mut self, values: &mut Vec<Expression>, preambles: &mut Vec<Statement>) {
        let length = values.len();
        let mut rewritten = Vec::with_capacity(length);
        for (index, value) in values.drain(..).enumerate() {
            let tail = index + 1 == length;
            rewritten.extend(self.rewrite(value, tail, preambles));
        }
        *values = rewritten;
    }

    fn rewrite_single(&mut self, expression: &mut Expression, preambles: &mut Vec<Statement>) {
        let value = std::mem::replace(expression, Expression::nil());
        let mut rewritten = self.rewrite(value, false, preambles);
        *expression = rewritten.remove(0);
    }

    fn rewrite(
        &mut self,
        expression: Expression,
        tail: bool,
        preambles: &mut Vec<Statement>,
    ) -> Vec<Expression> {
        if let Expression::Call(call) = &expression {
            if let Some(replacement) = self.try_inline(call, tail, preambles) {
                return replacement;
            }
        }
        let mut expression = expression;
        self.rewrite_children(&mut expression, preambles);
        vec![expression]
    }

    fn rewrite_children(&mut self, expression: &mut Expression, preambles: &mut Vec<Statement>) {
        match expression {
            Expression::Binary(binary) => {
                let mut left = binary.left().clone();
                let mut right = binary.right().clone();
                self.rewrite_single(&mut left, preambles);
                self.rewrite_single(&mut right, preambles);
                *binary.mutate_left() = left;
                *binary.mutate_right() = right;
            }
            Expression::Unary(unary) => {
                let mut inner = unary.get_expression().clone();
                self.rewrite_single(&mut inner, preambles);
                *unary.mutate_expression() = inner;
            }
            Expression::Parenthese(parenthese) => {
                let mut inner = parenthese.inner_expression().clone();
                self.rewrite_single(&mut inner, preambles);
                *parenthese.mutate_inner_expression() = inner;
            }
            Expression::Index(index) => {
                let mut prefix = index.get_prefix().clone();
                self.rewrite_prefix(&mut prefix, preambles);
                *index.mutate_prefix() = prefix;
                let mut key = index.get_index().clone();
                self.rewrite_single(&mut key, preambles);
                *index.mutate_index() = key;
            }
            Expression::Field(field) => {
                let mut prefix = field.get_prefix().clone();
                self.rewrite_prefix(&mut prefix, preambles);
                *field.mutate_prefix() = prefix;
            }
            Expression::Call(call) => {
                let mut inner = call.as_ref().clone();
                let mut prefix = inner.get_prefix().clone();
                self.rewrite_prefix(&mut prefix, preambles);
                let mut values: Vec<Expression> = inner.get_arguments().clone().to_expressions();
                self.rewrite_values(&mut values, preambles);
                *inner.mutate_prefix() = prefix;
                *inner.mutate_arguments() =
                    crate::nodes::Arguments::Tuple(crate::nodes::TupleArguments::from_iter(values));
                **call = inner;
            }
            Expression::If(if_expression) => {
                let mut condition = if_expression.get_condition().clone();
                self.rewrite_single(&mut condition, preambles);
                *if_expression.mutate_condition() = condition;
                let mut result = if_expression.get_result().clone();
                self.rewrite_single(&mut result, preambles);
                *if_expression.mutate_result() = result;
                for branch in if_expression.iter_mut_branches() {
                    let mut branch_condition = branch.get_condition().clone();
                    self.rewrite_single(&mut branch_condition, preambles);
                    *branch.mutate_condition() = branch_condition;
                    let mut branch_result = branch.get_result().clone();
                    self.rewrite_single(&mut branch_result, preambles);
                    *branch.mutate_result() = branch_result;
                }
                let mut else_result = if_expression.get_else_result().clone();
                self.rewrite_single(&mut else_result, preambles);
                *if_expression.mutate_else_result() = else_result;
            }
            Expression::Table(table) => {
                let entries: Vec<crate::nodes::TableEntry> =
                    table.iter_entries().cloned().collect();
                let length = entries.len();
                let mut replacement: Vec<crate::nodes::TableEntry> = Vec::with_capacity(length);
                for (index, entry) in entries.into_iter().enumerate() {
                    let tail = index + 1 == length;
                    match entry {
                        crate::nodes::TableEntry::Field(mut field) => {
                            let mut value = field.get_value().clone();
                            self.rewrite_single(&mut value, preambles);
                            *field.mutate_value() = value;
                            replacement.push(crate::nodes::TableEntry::Field(field));
                        }
                        crate::nodes::TableEntry::Index(mut index) => {
                            let mut key = index.get_key().clone();
                            self.rewrite_single(&mut key, preambles);
                            *index.mutate_key() = key;
                            let mut value = index.get_value().clone();
                            self.rewrite_single(&mut value, preambles);
                            *index.mutate_value() = value;
                            replacement.push(crate::nodes::TableEntry::Index(index));
                        }
                        crate::nodes::TableEntry::Value(value) => {
                            // The last array entry takes every value, like any
                            // other trailing value list.
                            for result in self.rewrite((*value).clone(), tail, preambles) {
                                replacement.push(crate::nodes::TableEntry::Value(Box::new(result)));
                            }
                        }
                    }
                }
                *table = crate::nodes::TableExpression::new(replacement);
            }
            Expression::Function(function) => self.rewrite_block(function.mutate_block()),
            Expression::TypeCast(type_cast) => {
                let mut inner = type_cast.get_expression().clone();
                self.rewrite_single(&mut inner, preambles);
                *type_cast.mutate_expression() = inner;
            }
            Expression::TypeInstantiation(instantiation) => {
                let mut prefix = instantiation.get_prefix().clone();
                self.rewrite_prefix(&mut prefix, preambles);
                *instantiation.mutate_prefix() = prefix;
            }
            _ => {}
        }
    }

    fn rewrite_prefix(&mut self, prefix: &mut Prefix, preambles: &mut Vec<Statement>) {
        match prefix {
            Prefix::Identifier(_) => {}
            Prefix::Field(field) => {
                let mut inner = field.get_prefix().clone();
                self.rewrite_prefix(&mut inner, preambles);
                *field.mutate_prefix() = inner;
            }
            Prefix::Index(index) => {
                let mut inner = index.get_prefix().clone();
                self.rewrite_prefix(&mut inner, preambles);
                *index.mutate_prefix() = inner;
                let mut key = index.get_index().clone();
                self.rewrite_single(&mut key, preambles);
                *index.mutate_index() = key;
            }
            Prefix::Call(call) => {
                let mut inner = call.as_ref().clone();
                let mut call_prefix = inner.get_prefix().clone();
                self.rewrite_prefix(&mut call_prefix, preambles);
                let mut values: Vec<Expression> = inner.get_arguments().clone().to_expressions();
                self.rewrite_values(&mut values, preambles);
                *inner.mutate_prefix() = call_prefix;
                *inner.mutate_arguments() =
                    crate::nodes::Arguments::Tuple(crate::nodes::TupleArguments::from_iter(values));
                // A call used as a prefix consumes one value.
                if let Some(mut replacement) = self.try_inline(&inner, false, preambles) {
                    let value = replacement.remove(0);
                    *prefix = Prefix::Parenthese(Box::new(
                        crate::nodes::ParentheseExpression::new(value),
                    ));
                } else {
                    **call = inner;
                }
            }
            Prefix::Parenthese(parenthese) => {
                let mut inner = parenthese.inner_expression().clone();
                self.rewrite_single(&mut inner, preambles);
                *parenthese.mutate_inner_expression() = inner;
            }
            Prefix::TypeInstantiation(instantiation) => {
                let mut inner = instantiation.get_prefix().clone();
                self.rewrite_prefix(&mut inner, preambles);
                *instantiation.mutate_prefix() = inner;
            }
        }
    }

    fn try_inline(
        &mut self,
        call: &crate::nodes::FunctionCall,
        tail: bool,
        preambles: &mut Vec<Statement>,
    ) -> Option<Vec<Expression>> {
        if call.get_method().is_some() {
            return None;
        }
        let Prefix::Identifier(identifier) = call.get_prefix() else {
            return None;
        };
        let symbol = self.model.resolve_identifier(identifier)?;
        let function = self.functions.get(&symbol)?;

        let scope = self.current_scope?;
        for upvalue in &function.upvalues {
            if !self.upvalue_is_reachable(*upvalue, scope) {
                return None;
            }
        }

        let results: Vec<String> = (0..function.arity).map(|_| self.names.generate()).collect();
        let done = self.names.generate();

        let mut bindings: Vec<TypedIdentifier> = function
            .parameters
            .iter()
            .map(|name| TypedIdentifier::new(name.clone()))
            .collect();
        let mut values = call.get_arguments().clone().to_expressions();
        while bindings.len() < values.len() {
            bindings.push(TypedIdentifier::new(self.names.generate()));
        }
        while values.len() < function.parameters.len() {
            values.push(Expression::nil());
        }

        let mut body = function.body.clone();
        rename_labels(&mut body, self.model, self.names);
        lower_returns(&mut body, &results, &done, 0);
        guard_loops(&mut body, &done, 0);

        let inner = Block::new(
            vec![
                Statement::from(VariableAssignment::new(bindings, values)),
                Statement::from(VariableAssignment::new(
                    vec![TypedIdentifier::new(done.clone())],
                    vec![Expression::from(false)],
                )),
                Statement::from(RepeatStatement::new(body, Expression::from(true))),
            ],
            None,
        );

        if !results.is_empty() {
            let declarations: Vec<TypedIdentifier> = results
                .iter()
                .map(|name| TypedIdentifier::new(name.clone()))
                .collect();
            preambles.push(Statement::from(VariableAssignment::new(
                declarations,
                Vec::new(),
            )));
        }
        preambles.push(Statement::from(DoStatement::new(inner)));
        *self.inlined += 1;

        let results: Vec<Expression> = results.into_iter().map(Expression::identifier).collect();
        if tail {
            Some(results)
        } else if let Some(first) = results.into_iter().next() {
            Some(vec![first])
        } else {
            Some(vec![Expression::nil()])
        }
    }

    fn upvalue_is_reachable(&self, symbol: SymbolId, scope: ScopeId) -> bool {
        let declaring = self.model.symbol(symbol).declaring_scope;
        let name = &self.model.symbol(symbol).name;
        let mut current = Some(scope);
        while let Some(id) = current {
            if id == declaring {
                return true;
            }
            if self
                .model
                .scope(id)
                .symbols
                .iter()
                .any(|other| self.model.symbol(*other).name == *name)
            {
                return false;
            }
            current = self.model.scope(id).parent;
        }
        false
    }
}

/// Gives every label in a body a fresh name, with its `goto`s.
fn rename_labels(block: &mut Block, model: &SemanticModel, names: &mut NameGenerator) {
    let mut renames: HashMap<LabelId, String> = HashMap::new();

    fn collect(
        block: &Block,
        model: &SemanticModel,
        renames: &mut HashMap<LabelId, String>,
        names: &mut NameGenerator,
    ) {
        for statement in block.iter_statements() {
            match statement {
                Statement::Label(label) => {
                    if let Some(id) = model.resolve_label(label.get_label()) {
                        renames.entry(id).or_insert_with(|| names.generate());
                    }
                }
                Statement::Do(statement) => collect(statement.get_block(), model, renames, names),
                Statement::While(statement) => {
                    collect(statement.get_block(), model, renames, names)
                }
                Statement::Repeat(statement) => {
                    collect(statement.get_block(), model, renames, names)
                }
                Statement::NumericFor(statement) => {
                    collect(statement.get_block(), model, renames, names)
                }
                Statement::GenericFor(statement) => {
                    collect(statement.get_block(), model, renames, names)
                }
                Statement::If(statement) => {
                    for branch in statement.iter_branches() {
                        collect(branch.get_block(), model, renames, names);
                    }
                    if let Some(else_block) = statement.get_else_block() {
                        collect(else_block, model, renames, names);
                    }
                }
                _ => {}
            }
        }
    }

    fn apply(block: &mut Block, model: &SemanticModel, renames: &HashMap<LabelId, String>) {
        for statement in block.iter_mut_statements() {
            match statement {
                Statement::Label(label) => {
                    if let Some(id) = model.resolve_label(label.get_label()) {
                        if let Some(name) = renames.get(&id) {
                            label.mutate_label().set_name(name.clone());
                        }
                    }
                }
                Statement::Goto(goto) => {
                    if let Some(id) = model.resolve_label(goto.get_label()) {
                        if let Some(name) = renames.get(&id) {
                            goto.mutate_label().set_name(name.clone());
                        }
                    }
                }
                Statement::Do(statement) => apply(statement.mutate_block(), model, renames),
                Statement::While(statement) => apply(statement.mutate_block(), model, renames),
                Statement::Repeat(statement) => apply(statement.mutate_block(), model, renames),
                Statement::NumericFor(statement) => apply(statement.mutate_block(), model, renames),
                Statement::GenericFor(statement) => apply(statement.mutate_block(), model, renames),
                Statement::If(statement) => {
                    for branch in statement.mutate_branches() {
                        apply(branch.mutate_block(), model, renames);
                    }
                    if let Some(else_block) = statement.mutate_else_block() {
                        apply(else_block, model, renames);
                    }
                }
                _ => {}
            }
        }
    }

    collect(block, model, &mut renames, names);
    apply(block, model, &renames);
}

/// Lowers every `return` to a result assignment and a `break`.
fn lower_returns(block: &mut Block, results: &[String], done: &str, depth: usize) {
    let mut statements = block.take_statements();
    for statement in statements.iter_mut() {
        lower_statement_returns(statement, results, done, depth);
    }
    block.set_statements(statements);

    let Some(last) = block.get_last_statement().cloned() else {
        return;
    };
    let LastStatement::Return(return_statement) = last else {
        return;
    };
    if depth != 0 {
        return;
    }

    let values: Vec<Expression> = return_statement.iter_expressions().cloned().collect();
    let mut statements = block.take_statements();
    if !results.is_empty() {
        let variables: Vec<Variable> = results
            .iter()
            .map(|name| Variable::Identifier(Identifier::new(name.clone())))
            .collect();
        statements.push(Statement::from(AssignStatement::new(variables, values)));
    }
    statements.push(Statement::from(AssignStatement::new(
        vec![Variable::Identifier(Identifier::new(done.to_owned()))],
        vec![Expression::from(true)],
    )));
    block.set_statements(statements);
    block.set_last_statement(LastStatement::Break(None));
}

fn lower_statement_returns(
    statement: &mut Statement,
    results: &[String],
    done: &str,
    depth: usize,
) {
    match statement {
        Statement::Do(statement) => lower_returns(statement.mutate_block(), results, done, depth),
        Statement::If(statement) => {
            for branch in statement.mutate_branches() {
                lower_returns(branch.mutate_block(), results, done, depth);
            }
            if let Some(else_block) = statement.mutate_else_block() {
                lower_returns(else_block, results, done, depth);
            }
        }
        Statement::While(statement) => {
            lower_returns(statement.mutate_block(), results, done, depth)
        }
        Statement::Repeat(statement) => {
            lower_returns(statement.mutate_block(), results, done, depth)
        }
        Statement::NumericFor(statement) => {
            lower_returns(statement.mutate_block(), results, done, depth)
        }
        Statement::GenericFor(statement) => {
            lower_returns(statement.mutate_block(), results, done, depth)
        }
        Statement::Function(statement) => {
            lower_returns(statement.mutate_block(), results, done, depth + 1)
        }
        Statement::LocalFunction(statement) => {
            lower_returns(statement.mutate_block(), results, done, depth + 1)
        }
        Statement::TypeFunction(statement) => {
            lower_returns(statement.mutate_block(), results, done, depth + 1)
        }
        _ => {}
    }
}

/// Inserts `if done then break end` after every loop at the body's own level.
fn guard_loops(block: &mut Block, done: &str, depth: usize) {
    let mut statements = block.take_statements();
    let mut guarded = Vec::with_capacity(statements.len() * 2);
    for mut statement in statements.drain(..) {
        match &mut statement {
            Statement::Do(inner) => guard_loops(inner.mutate_block(), done, depth),
            Statement::If(inner) => {
                for branch in inner.mutate_branches() {
                    guard_loops(branch.mutate_block(), done, depth);
                }
                if let Some(else_block) = inner.mutate_else_block() {
                    guard_loops(else_block, done, depth);
                }
            }
            Statement::While(inner) => guard_loops(inner.mutate_block(), done, depth),
            Statement::Repeat(inner) => guard_loops(inner.mutate_block(), done, depth),
            Statement::NumericFor(inner) => guard_loops(inner.mutate_block(), done, depth),
            Statement::GenericFor(inner) => guard_loops(inner.mutate_block(), done, depth),
            Statement::Function(inner) => guard_loops(inner.mutate_block(), done, depth + 1),
            Statement::LocalFunction(inner) => guard_loops(inner.mutate_block(), done, depth + 1),
            Statement::TypeFunction(inner) => guard_loops(inner.mutate_block(), done, depth + 1),
            _ => {}
        }
        let is_loop = matches!(
            statement,
            Statement::While(_)
                | Statement::Repeat(_)
                | Statement::NumericFor(_)
                | Statement::GenericFor(_)
        );
        guarded.push(statement);
        if is_loop && depth == 0 {
            guarded.push(guard(done));
        }
    }
    block.set_statements(guarded);
}

fn guard(done: &str) -> Statement {
    let condition = Expression::identifier(done.to_owned());
    let block = Block::new(vec![], Some(LastStatement::Break(None)));
    Statement::from(crate::nodes::IfStatement::new(
        vec![crate::nodes::IfBranch::new(condition, block)],
        None,
    ))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate, parse};

    fn inlined_output(code: &str) -> String {
        let (mut block, model) = analyze(code);
        inline_local_functions(&mut block, &model);
        generate(&block)
    }

    #[test]
    fn inlines_a_body_with_statements() {
        let output = inlined_output(
            "local function add(a, b)\n    local sum = a + b\n    return sum\nend\nreturn add(1, 2)",
        );
        assert!(!output.contains("add(1, 2)"), "{output}");
        assert!(output.contains("local sum = a + b"), "{output}");
    }

    #[test]
    fn inlines_a_body_with_control_flow() {
        let output = inlined_output(
            "local function sign(value)\n    if value > 0 then\n        return 1\n    end\n    return -1\nend\nreturn sign(3)",
        );
        assert!(!output.contains("sign(3)"), "{output}");
        assert!(output.contains("return 1"), "{output}");
    }

    #[test]
    fn inlines_a_body_with_a_loop() {
        let output = inlined_output(
            "local function sum(n)\n    local total = 0\n    for i = 1, n do\n        total = total + i\n    end\n    return total\nend\nreturn sum(3)",
        );
        assert!(!output.contains("sum(3)"), "{output}");
    }

    #[test]
    fn inlines_a_return_inside_a_loop() {
        let output = inlined_output(
            "local function find(n)\n    for i = 1, n do\n        if i == 2 then\n            return i\n        end\n    end\n    return -1\nend\nreturn find(5)",
        );
        assert!(!output.contains("find(5)"), "{output}");
    }

    #[test]
    fn inlines_a_body_that_falls_through() {
        let output = inlined_output("local function show(x)\n    print(x)\nend\nshow(1)\nreturn 0");
        assert!(!output.contains("show(1)"), "{output}");
        assert!(output.contains("print(x)"), "{output}");
    }

    #[test]
    fn inlines_multi_value_returns() {
        let output = inlined_output(
            "local function pair(x)\n    return x, x\nend\nlocal a, b = pair(1)\nreturn a, b",
        );
        assert!(!output.contains("pair(1)"), "{output}");
    }

    #[test]
    fn inlines_an_upvalue_read() {
        let output = inlined_output(
            "local y = 2\nlocal function add(x)\n    return x + y\nend\nreturn add(1)",
        );
        assert!(!output.contains("add(1)"), "{output}");
    }

    #[test]
    fn does_not_inline_a_shadowed_upvalue() {
        let output = inlined_output(
            "local y = 2\nlocal function add(x)\n    return x + y\nend\ndo\n    local y = 3\n    return add(1)\nend",
        );
        assert!(output.contains("add(1)"), "{output}");
    }

    #[test]
    fn does_not_inline_a_body_that_uses_varargs() {
        let output = inlined_output("local function f()\n    return ...\nend\nreturn f(1)");
        assert!(output.contains("f(1)"), "{output}");
    }

    #[test]
    fn inlines_a_variadic_callee_that_ignores_its_varargs() {
        let output = inlined_output(
            "local function first(value, ...)\n    return value\nend\nreturn first(1, 2, 3)",
        );
        assert!(!output.contains("first(1, 2, 3)"), "{output}");
    }

    #[test]
    fn evaluates_arguments_once_however_often_a_parameter_is_used() {
        let output = inlined_output(
            "local function twice(value)\n    return value + value\nend\nreturn twice(bump())",
        );
        assert_eq!(output.matches("bump()").count(), 1, "{output}");
    }

    #[test]
    fn inlines_gotos_and_labels_without_colliding() {
        let output = inlined_output(
            "local function pick(x)\n    if x > 0 then\n        goto positive\n    end\n    do\n        return -1\n    end\n    ::positive::\n    return 1\nend\nreturn pick(1)",
        );
        assert!(!output.contains("pick(1)"), "{output}");
    }

    #[test]
    fn inlines_a_recursive_function_once_without_looping() {
        let output = inlined_output(
            "local function count_down(n)\n    if n > 0 then\n        return count_down(n - 1) + 0\n    end\n    return 0\nend\nreturn count_down(3)",
        );
        assert!(!output.contains("count_down(3)"), "{output}");
    }

    #[test]
    fn inlines_multi_value_return_into_a_table() {
        let output = inlined_output(
            "local function pair()\n    return 1, 2\nend\nlocal t = { pair() }\nreturn #t",
        );
        assert!(!output.contains("{ pair() }"), "{output}");
        parse(&output);
    }

    #[test]
    fn inlines_a_call_used_as_a_prefix() {
        let output = inlined_output(
            "local function make(value)\n    return function() return value end\nend\nreturn make(7)()",
        );
        assert!(!output.contains("make(7)"), "{output}");
    }

    #[test]
    fn the_inlined_output_reparses() {
        for code in [
            "local function add(a, b)\n    local sum = a + b\n    return sum\nend\nreturn add(1, 2)",
            "local function sign(value)\n    if value > 0 then\n        return 1\n    end\n    return -1\nend\nreturn sign(3)",
            "local function sum(n)\n    local total = 0\n    for i = 1, n do\n        total = total + i\n    end\n    return total\nend\nreturn sum(3)",
            "local function find(n)\n    for i = 1, n do\n        if i == 2 then\n            return i\n        end\n    end\n    return -1\nend\nreturn find(5)",
            "local function show(x)\n    print(x)\nend\nshow(1)\nreturn 0",
            "local function pair(x)\n    return x, x\nend\nlocal a, b = pair(1)\nreturn a, b",
            "local function first(value, ...)\n    return value\nend\nreturn first(1, 2, 3)",
            "local function pick(x)\n    if x > 0 then\n        goto positive\n    end\n    do\n        return -1\n    end\n    ::positive::\n    return 1\nend\nreturn pick(1)",
            "local function f(a)\n    return a\nend\nreturn f(1, 2, 3)",
            "local function f(a)\n    return a\nend\nlocal x = f(1)\nreturn x",
            "local function pair()\n    return 1, 2\nend\nlocal t = { pair() }\nreturn #t",
            "local function make(value)\n    return function() return value end\nend\nreturn make(7)()",
        ] {
            let output = inlined_output(code);
            parse(&output);
        }
    }
}
