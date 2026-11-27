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
//! The preamble runs at the start of the statement, so a call is inlined only
//! when everything evaluated before it in that statement is inert or can be
//! stored first. Earlier list values, table entries, and eagerly-read operands
//! are stored in fresh locals in their original order, and a call's callee is
//! stored before an inlined argument. An inlined call's own arguments are
//! rewritten first, so nested calls are inlined too.
//!
//! A bare local of the current function is read by the binary operator, index,
//! or method call itself, after the other operand runs, so it is left in place
//! instead of being stored early. Concatenation operands and globals, upvalues,
//! and compound expressions are read eagerly and are stored first.
//!
//! A short-circuit operand, an `if` expression branch, an `elseif` condition, a
//! `while`/`repeat` condition, and a method receiver still stop further
//! inlining. A function that contains a `goto` or label refuses inlining
//! entirely, because the preamble declares locals that a forward jump could
//! enter.
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
//! * a call is left alone when its callee cannot be resolved to a local
//!   function that is bound before the call.
//!
//! A function may also be bound by a declaration and a later assignment
//! (`local f` followed by `f = function() ... end`). Such a call is inlined
//! only when the assignment provably runs before it: the assignment's block
//! must contain the call, or contain the statement that leads to it, with the
//! call after the assignment. A `goto` or label in the same function defeats
//! that proof, because a backward jump can reach a call before the assignment.
//!
//! Everything else — statements, `if`, loops, `return` inside loops,
//! fall-through, any number of results, upvalue reads, immutable aliases, and
//! `goto`/labels in bodies — is handled.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::cfg::{validate_block_identities, StatementOrigin, UnidentifiedBlock};
use crate::nodes::{
    Arguments, AssignStatement, BinaryOperator, Block, BlockIdentity, ChildVisitor, DoStatement,
    Expression, Identifier, LastStatement, Prefix, ReadOnlyChildren, RepeatStatement, Statement,
    TableExpression, TypedIdentifier, Variable, VariableAssignment,
};
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
    /// Whether every call yields exactly [`Self::arity`] values.
    uniform: bool,
    /// The symbols the body reads from outside the function.
    upvalues: Vec<SymbolId>,
    /// The `f = function ...` statement that binds the function, when the
    /// declaration and the assignment are separate. Calls are inlined only
    /// where this assignment provably runs first.
    assignment: Option<StatementOrigin>,
}

/// An error produced while inlining local functions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineError {
    /// A block has no structural identity, so statement positions cannot be
    /// trusted.
    Provenance(UnidentifiedBlock),
}

impl fmt::Display for InlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provenance(error) => write!(f, "cannot inline local functions: {error}"),
        }
    }
}

impl std::error::Error for InlineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Provenance(error) => Some(error),
        }
    }
}

impl From<UnidentifiedBlock> for InlineError {
    fn from(error: UnidentifiedBlock) -> Self {
        Self::Provenance(error)
    }
}

/// Replaces calls to inlinable local functions by their body.
///
/// Returns the number of calls inlined.
///
/// # Errors
///
/// Returns [`InlineError::Provenance`] when a block has no structural identity,
/// which means the tree was not produced by the semantic model it is being
/// analyzed with.
pub fn inline_local_functions(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, InlineError> {
    validate_block_identities(block)?;

    let mut collector = FunctionCollector::new(model);
    collector.walk(block);

    if collector.functions.is_empty() {
        return Ok(0);
    }

    let mut names = NameGenerator::new(model);
    let mut inlined = 0;
    let mut inliner = Inliner {
        model,
        functions: &collector.functions,
        parents: &collector.parents,
        declarations: &collector.declarations,
        aliases: &collector.aliases,
        labelled: &collector.labelled,
        block_functions: &collector.block_functions,
        unconditional: &collector.unconditional,
        names: &mut names,
        current_scope: None,
        current_position: None,
        clean: false,
        inlined: &mut inlined,
    };
    inliner.rewrite_block(block);

    Ok(inlined)
}

// ----------------------------------------------------------------------
// Deciding what can be inlined
// ----------------------------------------------------------------------

fn inlinable(
    model: &SemanticModel,
    symbol: SymbolId,
    body: &Block,
    parameters: &[&TypedIdentifier],
    assignment: Option<StatementOrigin>,
) -> Option<InlinableFunction> {
    let info = model.symbol(symbol);
    if info.kind != SymbolKind::Local {
        return None;
    }
    match assignment {
        // The declaration plus this one assignment is the whole history of the
        // symbol. The call-site check proves the assignment has already run; a
        // second write could have replaced the function before the call.
        Some(_) if info.write_count == 2 => {}
        Some(_) => return None,
        // A declaration binds the function before the symbol is in scope, so
        // every call already sees it.
        None if model.is_immutable(symbol) => {}
        None => return None,
    }

    let mut check = BodyCheck {
        model,
        scope: model.try_scope_of(body).ok()?,
        loop_depth: 0,
        function_depth: 0,
        arity: None,
        uniform: true,
        upvalues: Vec::new(),
        rejects: false,
    };
    check.visit(body);

    if check.rejects {
        return None;
    }

    let arity = check.arity.unwrap_or(0);
    let terminates = matches!(body.get_last_statement(), Some(LastStatement::Return(_)));
    let uniform = check.uniform && (arity == 0 || terminates);

    Some(InlinableFunction {
        parameters: parameters
            .iter()
            .map(|parameter| parameter.get_identifier().get_name().clone())
            .collect(),
        body: body.clone(),
        arity,
        uniform,
        upvalues: check.upvalues,
        assignment,
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
    /// Whether every `return` has the same expression count and none is a call.
    uniform: bool,
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
            Statement::Function(statement) => {
                self.visit_identifier(statement.get_name().get_name());
                self.visit_function(statement.get_block());
            }
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
            Variable::Identifier(identifier) => self.visit_identifier(identifier),
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

        let expressions: Vec<&Expression> = return_statement.iter_expressions().collect();
        if self.function_depth != 0 {
            // A nested function's return does not shape this body's arity, but
            // its expressions still reference names that must resolve at the
            // call site.
            for expression in expressions {
                self.visit_expression(expression);
            }
            return;
        }
        for expression in &expressions {
            if matches!(expression, Expression::Call(_)) {
                // A returned call yields that call's values, so the count is not
                // fixed by the expression list.
                self.uniform = false;
            }
            self.visit_expression(expression);
        }

        match self.arity {
            None => self.arity = Some(expressions.len()),
            Some(arity) => {
                if arity != expressions.len() {
                    self.uniform = false;
                }
                self.arity = Some(arity.max(expressions.len()));
            }
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
            Expression::Identifier(identifier) => self.visit_identifier(identifier),
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
            Expression::TypeInstantiation(instantiation) => {
                self.visit_prefix(instantiation.get_prefix());
            }
            _ => {}
        }
    }

    fn visit_identifier(&mut self, identifier: &Identifier) {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            self.rejects = true;
            return;
        };
        if !self.is_declared_in_function(symbol) && !self.upvalues.contains(&symbol) {
            self.upvalues.push(symbol);
        }
    }

    fn visit_prefix(&mut self, prefix: &Prefix) {
        match prefix {
            Prefix::Identifier(identifier) => self.visit_identifier(identifier),
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

/// Gathers the functions that can be inlined, the position of every function
/// assignment, the enclosing statement of every block, and the function scopes
/// that contain a `goto` or label.
struct FunctionCollector<'m> {
    model: &'m SemanticModel,
    functions: HashMap<SymbolId, InlinableFunction>,
    /// The statement that evaluates each local's initializer.
    declarations: HashMap<SymbolId, StatementOrigin>,
    /// A local whose only non-declaration write is `alias = target`.
    aliases: HashMap<SymbolId, (SymbolId, StatementOrigin)>,
    /// The statement that contains each AST block.
    parents: HashMap<BlockIdentity, (BlockIdentity, u32)>,
    /// Function scopes whose bodies contain a `goto` or label.
    labelled: HashSet<ScopeId>,
    /// The function scope that owns each block.
    block_functions: HashMap<BlockIdentity, ScopeId>,
    /// Blocks that are the body of a `do` statement. Such a body always runs to
    /// completion, so an assignment at its top level completes with the `do`.
    unconditional: HashSet<BlockIdentity>,
}

impl<'m> FunctionCollector<'m> {
    fn new(model: &'m SemanticModel) -> Self {
        Self {
            model,
            functions: HashMap::new(),
            declarations: HashMap::new(),
            aliases: HashMap::new(),
            parents: HashMap::new(),
            labelled: HashSet::new(),
            block_functions: HashMap::new(),
            unconditional: HashSet::new(),
        }
    }

    fn walk(&mut self, block: &Block) {
        let Some(identity) = block.get_identity() else {
            return;
        };
        let function = self.function_scope(block);
        if let Some(function) = function {
            self.block_functions.insert(identity, function);
        }
        let count = block.statements_len();
        for (index, statement) in block.iter_statements().enumerate() {
            let origin = StatementOrigin {
                block: identity,
                index: index as u32,
            };
            self.record_statement(statement, origin);
            if matches!(statement, Statement::Goto(_) | Statement::Label(_)) {
                if let Some(function) = function {
                    self.labelled.insert(function);
                }
            }
            if let Statement::Do(statement) = statement {
                if let Some(identity) = statement.get_block().get_identity() {
                    self.unconditional.insert(identity);
                }
            }
            statement.for_each_child(&mut ChildWalker {
                collector: self,
                parent: origin,
            });
        }
        if let Some(last) = block.get_last_statement() {
            let origin = StatementOrigin {
                block: identity,
                index: count as u32,
            };
            last.for_each_child(&mut ChildWalker {
                collector: self,
                parent: origin,
            });
        }
    }

    fn record_statement(&mut self, statement: &Statement, origin: StatementOrigin) {
        match statement {
            Statement::LocalFunction(function) => {
                let parameters: Vec<&TypedIdentifier> = function.iter_parameters().collect();
                self.record(
                    function.get_identifier(),
                    function.get_block(),
                    &parameters,
                    None,
                );
            }
            Statement::LocalAssign(assignment) => {
                if assignment.variables_len() == 1 && assignment.values_len() == 1 {
                    if let (Some(variable), Some(Expression::Function(function))) = (
                        assignment.iter_variables().next(),
                        assignment.iter_values().next(),
                    ) {
                        let parameters: Vec<&TypedIdentifier> =
                            function.iter_parameters().collect();
                        self.record(
                            variable.get_identifier(),
                            function.get_block(),
                            &parameters,
                            None,
                        );
                    }
                }
                for (variable, value) in assignment.iter_variables().zip(assignment.iter_values()) {
                    if !matches!(value, Expression::Identifier(_)) {
                        continue;
                    }
                    if let Some(symbol) = self.model.resolve_identifier(variable.get_identifier()) {
                        self.declarations.entry(symbol).or_insert(origin);
                    }
                }
            }
            Statement::Assign(assignment) => {
                if assignment.variables_len() != 1 || assignment.values_len() != 1 {
                    return;
                }
                let Some(Variable::Identifier(identifier)) = assignment.iter_variables().next()
                else {
                    return;
                };
                match assignment.iter_values().next() {
                    Some(Expression::Function(function)) => {
                        let parameters: Vec<&TypedIdentifier> =
                            function.iter_parameters().collect();
                        self.record(identifier, function.get_block(), &parameters, Some(origin));
                    }
                    Some(Expression::Identifier(target)) => {
                        let Some(symbol) = self.model.resolve_identifier(identifier) else {
                            return;
                        };
                        let Some(target) = self.model.resolve_identifier(target) else {
                            return;
                        };
                        let info = self.model.symbol(symbol);
                        if info.kind == SymbolKind::Local && info.write_count == 2 {
                            self.aliases.insert(symbol, (target, origin));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn record(
        &mut self,
        identifier: &Identifier,
        body: &Block,
        parameters: &[&TypedIdentifier],
        assignment: Option<StatementOrigin>,
    ) {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return;
        };
        if let Some(function) = inlinable(self.model, symbol, body, parameters, assignment) {
            self.functions.insert(symbol, function);
        }
    }

    fn walk_child(&mut self, child: &Block, parent: StatementOrigin) {
        if let Some(identity) = child.get_identity() {
            self.parents.insert(identity, (parent.block, parent.index));
        }
        self.walk(child);
    }

    fn function_scope(&self, block: &Block) -> Option<ScopeId> {
        let scope = self.model.try_scope_of(block).ok()?;
        self.function_boundary(scope)
    }

    fn function_boundary(&self, mut scope: ScopeId) -> Option<ScopeId> {
        loop {
            let current = self.model.scope(scope);
            if current.kind.is_function_boundary() {
                return Some(scope);
            }
            scope = current.parent?;
        }
    }
}

/// Finds every block nested in a statement's expressions, recording the
/// statement as its parent. Function expressions are the only expressions that
/// own a block, but they can be nested in any expression position.
struct ChildWalker<'a, 'm> {
    collector: &'a mut FunctionCollector<'m>,
    parent: StatementOrigin,
}

impl ChildVisitor for ChildWalker<'_, '_> {
    fn block(&mut self, block: &Block) {
        self.collector.walk_child(block, self.parent);
    }

    fn expression(&mut self, expression: &Expression) {
        expression.for_each_child(self);
    }

    fn prefix(&mut self, prefix: &Prefix) {
        prefix.for_each_child(self);
    }

    fn variable(&mut self, variable: &Variable) {
        variable.for_each_child(self);
    }

    fn table(&mut self, table: &TableExpression) {
        table.for_each_child(self);
    }

    fn arguments(&mut self, arguments: &Arguments) {
        arguments.for_each_child(self);
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
    /// The statement that contains each nested block.
    parents: &'m HashMap<BlockIdentity, (BlockIdentity, u32)>,
    /// The statement that evaluates each local's initializer.
    declarations: &'m HashMap<SymbolId, StatementOrigin>,
    /// A local whose only non-declaration write is `alias = target`.
    aliases: &'m HashMap<SymbolId, (SymbolId, StatementOrigin)>,
    /// Function scopes whose bodies contain a `goto` or label.
    labelled: &'m HashSet<ScopeId>,
    /// The function scope that owns each block.
    block_functions: &'m HashMap<BlockIdentity, ScopeId>,
    /// Blocks that are the body of a `do` statement.
    unconditional: &'m HashSet<BlockIdentity>,
    names: &'n mut NameGenerator,
    /// The scope of the block currently being rewritten.
    current_scope: Option<ScopeId>,
    /// The statement being rewritten, or the position after the block's last
    /// statement when rewriting its last statement.
    current_position: Option<StatementOrigin>,
    /// Whether the statement has not evaluated anything observable yet, so a
    /// call can be moved to the statement's start without reordering.
    clean: bool,
    inlined: &'i mut usize,
}

impl Inliner<'_, '_, '_> {
    fn rewrite_block(&mut self, block: &mut Block) {
        let previous_scope = self.current_scope;
        let previous_position = self.current_position;
        let previous_clean = self.clean;
        let scope = self.model.try_scope_of(block).ok();
        self.current_scope = scope;
        let identity = block.get_identity();
        let statements = block.take_statements();
        let count = statements.len();
        let mut rewritten = Vec::with_capacity(count);
        for (index, mut statement) in statements.into_iter().enumerate() {
            self.current_position = identity.map(|block| StatementOrigin {
                block,
                index: index as u32,
            });
            self.clean = true;
            let mut preambles = Vec::new();
            self.rewrite_statement(&mut statement, &mut preambles);
            rewritten.append(&mut preambles);
            rewritten.push(statement);
        }
        block.set_statements(rewritten);

        if let Some(mut last) = block.take_last_statement() {
            self.current_position = identity.map(|block| StatementOrigin {
                block,
                index: count as u32,
            });
            self.clean = true;
            let mut preambles = Vec::new();
            self.rewrite_last_statement(&mut last, &mut preambles);
            if !preambles.is_empty() {
                let mut statements = block.take_statements();
                statements.append(&mut preambles);
                block.set_statements(statements);
            }
            block.set_last_statement(last);
        }
        self.current_scope = previous_scope;
        self.current_position = previous_position;
        self.clean = previous_clean;
    }

    fn rewrite_statement(&mut self, statement: &mut Statement, preambles: &mut Vec<Statement>) {
        match statement {
            Statement::Assign(assignment) => {
                let variables: Vec<Variable> = assignment.iter_variables().cloned().collect();
                let evaluated_variables = variables
                    .iter()
                    .any(|variable| !matches!(variable, Variable::Identifier(_)));
                let mut rewritten_variables = Vec::with_capacity(variables.len());
                for mut variable in variables {
                    self.rewrite_variable(&mut variable, preambles);
                    rewritten_variables.push(variable);
                }
                if evaluated_variables {
                    // The variable expressions are evaluated before the values.
                    self.clean = false;
                }
                let mut values: Vec<Expression> = assignment.iter_values().cloned().collect();
                let multi_tail = rewritten_variables.len() >= 2;
                self.rewrite_values(&mut values, multi_tail, preambles);
                *assignment = AssignStatement::new(rewritten_variables, values);
            }
            Statement::LocalAssign(assignment) => {
                let variables: Vec<TypedIdentifier> =
                    assignment.iter_variables().cloned().collect();
                let mut values: Vec<Expression> = assignment.iter_values().cloned().collect();
                let multi_tail = variables.len() >= 2;
                self.rewrite_values(&mut values, multi_tail, preambles);
                *assignment = VariableAssignment::new(variables, values);
            }
            Statement::Call(slot) => {
                let mut call = slot.clone();
                if self.try_inline(&call, false, preambles).is_some() {
                    // The call's result is discarded; the body ran in the preamble.
                    *statement = Statement::Do(DoStatement::new(Block::new(Vec::new(), None)));
                } else {
                    self.rewrite_call_contents(&mut call, preambles);
                    *slot = call;
                }
            }
            Statement::CompoundAssign(assignment) => {
                self.clean = false;
                let mut variable = assignment.get_variable().clone();
                self.rewrite_variable(&mut variable, preambles);
                *assignment.mutate_variable() = variable;
                let mut value = assignment.get_value().clone();
                self.rewrite_single(&mut value, preambles);
                *assignment.mutate_value() = value;
            }
            Statement::If(statement) => {
                for (index, branch) in statement.mutate_branches().iter_mut().enumerate() {
                    if index > 0 {
                        // An `elseif` condition only runs when the earlier
                        // conditions were false.
                        self.clean = false;
                    }
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
                // The condition runs before every iteration, so its evaluation
                // cannot be hoisted to the statement.
                self.clean = false;
                let mut condition = statement.get_condition().clone();
                self.rewrite_single(&mut condition, preambles);
                statement.mutate_condition().clone_from(&condition);
                self.rewrite_block(statement.mutate_block());
            }
            Statement::Repeat(statement) => {
                self.rewrite_block(statement.mutate_block());
                // The condition runs after every iteration, so its evaluation
                // cannot be hoisted to the statement.
                self.clean = false;
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
                self.rewrite_values(&mut values, true, preambles);
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
        self.rewrite_values(&mut values, true, preambles);
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

    fn rewrite_values(
        &mut self,
        values: &mut Vec<Expression>,
        multi_tail: bool,
        preambles: &mut Vec<Statement>,
    ) {
        let length = values.len();
        let original = std::mem::take(values);
        let mut rewritten = Vec::with_capacity(length);
        for (index, value) in original.iter().enumerate() {
            let later_may_inline = original[index + 1..]
                .iter()
                .any(|later| self.expression_may_inline(later));
            let clean = self.clean;
            let tail = multi_tail && index + 1 == length;
            let results = self.rewrite(value.clone(), tail, preambles);
            if later_may_inline && clean && self.position_allows_insertion() {
                // Store this value before the later body runs; the stored value
                // keeps the original evaluation order.
                for result in results {
                    rewritten.push(self.materialize(result, preambles));
                }
                self.clean = true;
            } else {
                rewritten.extend(results);
            }
        }
        *values = rewritten;
    }

    /// Rewrites a call's callee and arguments. Earlier arguments are stored in
    /// fresh locals when a later argument can be inlined, so the inline body
    /// still runs after them.
    fn rewrite_call_contents(
        &mut self,
        call: &mut crate::nodes::FunctionCall,
        preambles: &mut Vec<Statement>,
    ) {
        let arguments: Vec<Expression> = call.get_arguments().clone().to_expressions();
        let method = call.get_method().is_some();
        let spill = !method
            && arguments
                .iter()
                .any(|argument| self.expression_may_inline(argument));
        let prefix_clean = self.clean;
        let mut prefix = call.get_prefix().clone();
        self.rewrite_prefix(&mut prefix, preambles);
        if spill && prefix_clean && self.position_allows_insertion() {
            prefix = self.materialize_prefix(prefix, preambles);
            self.clean = true;
        }
        *call.mutate_prefix() = prefix;
        let mut values = arguments;
        self.rewrite_values(&mut values, true, preambles);
        *call.mutate_arguments() =
            crate::nodes::Arguments::Tuple(crate::nodes::TupleArguments::from_iter(values));
    }

    /// Stores `expression` in a fresh local, preserving when it is evaluated.
    fn materialize(
        &mut self,
        expression: Expression,
        preambles: &mut Vec<Statement>,
    ) -> Expression {
        if expression_is_inert(&expression) {
            return expression;
        }
        let name = self.names.generate();
        preambles.push(Statement::from(VariableAssignment::new(
            vec![TypedIdentifier::new(name.clone())],
            vec![expression],
        )));
        Expression::identifier(name)
    }

    fn materialize_prefix(&mut self, prefix: Prefix, preambles: &mut Vec<Statement>) -> Prefix {
        let expression = prefix_into_expression(prefix);
        match self.materialize(expression, preambles) {
            Expression::Identifier(identifier) => Prefix::Identifier(identifier),
            expression => Prefix::Parenthese(Box::new(crate::nodes::ParentheseExpression::new(
                expression,
            ))),
        }
    }

    /// Returns whether a new local can be declared at the current position
    /// without disturbing a `goto`.
    fn position_allows_insertion(&self) -> bool {
        let Some(position) = self.current_position else {
            return false;
        };
        let Some(function) = self.block_functions.get(&position.block) else {
            return false;
        };
        !self.labelled.contains(function)
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
        if !expression_is_inert(&expression) {
            self.clean = false;
        }
        vec![expression]
    }

    fn rewrite_children(&mut self, expression: &mut Expression, preambles: &mut Vec<Statement>) {
        match expression {
            Expression::Binary(binary) => {
                let conditional =
                    matches!(binary.operator(), BinaryOperator::And | BinaryOperator::Or);
                // Concatenation copies every operand into a register before the
                // operation, so its left operand is read eagerly.
                let concat = matches!(binary.operator(), BinaryOperator::Concat);
                let right_may_inline = self.expression_may_inline(binary.right());
                let clean = self.clean;
                let mut left = binary.left().clone();
                self.rewrite_single(&mut left, preambles);
                if !conditional && right_may_inline && clean && self.position_allows_insertion() {
                    // A current-function local is read by the operator after the
                    // other operand runs, so it must not be copied early.
                    if concat || !self.is_delayed_operand(&left) {
                        left = self.materialize(left, preambles);
                    }
                    self.clean = true;
                }
                *binary.mutate_left() = left;
                if conditional {
                    // The right operand only runs when the left one does not
                    // decide the result.
                    self.clean = false;
                }
                let mut right = binary.right().clone();
                self.rewrite_single(&mut right, preambles);
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
                let key_may_inline = self.expression_may_inline(index.get_index());
                let clean = self.clean;
                let mut prefix = index.get_prefix().clone();
                self.rewrite_prefix(&mut prefix, preambles);
                if key_may_inline && clean && self.position_allows_insertion() {
                    // A current-function local is read by the index operation
                    // after the key runs, so it must not be copied early.
                    if !self.prefix_is_delayed(&prefix) {
                        prefix = self.materialize_prefix(prefix, preambles);
                    }
                    self.clean = true;
                }
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
            Expression::Call(call) => self.rewrite_call_contents(call, preambles),
            Expression::If(if_expression) => {
                let mut condition = if_expression.get_condition().clone();
                self.rewrite_single(&mut condition, preambles);
                *if_expression.mutate_condition() = condition;
                // Only one of the result branches runs.
                self.clean = false;
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
                // An entry with a computed key keeps the key in a register until
                // the store, so earlier entries cannot be copied out of the way.
                let spillable = entries.iter().all(|entry| match entry {
                    crate::nodes::TableEntry::Index(index) => {
                        expression_is_literal_key(index.get_key())
                    }
                    _ => true,
                });
                let length = entries.len();
                let mut replacement: Vec<crate::nodes::TableEntry> = Vec::with_capacity(length);
                for (index, entry) in entries.iter().enumerate() {
                    let later_may_inline = spillable
                        && entries[index + 1..]
                            .iter()
                            .any(|later| self.table_entry_may_inline(later));
                    let clean = self.clean;
                    let tail = index + 1 == length;
                    match entry.clone() {
                        crate::nodes::TableEntry::Field(mut field) => {
                            let mut value = field.get_value().clone();
                            self.rewrite_single(&mut value, preambles);
                            if later_may_inline && clean && self.position_allows_insertion() {
                                value = self.materialize(value, preambles);
                                self.clean = true;
                            }
                            *field.mutate_value() = value;
                            replacement.push(crate::nodes::TableEntry::Field(field));
                        }
                        crate::nodes::TableEntry::Index(mut index) => {
                            let mut key = index.get_key().clone();
                            self.rewrite_single(&mut key, preambles);
                            if spillable
                                && later_may_inline
                                && clean
                                && self.position_allows_insertion()
                            {
                                key = self.materialize(key, preambles);
                                self.clean = true;
                            }
                            *index.mutate_key() = key;
                            let mut value = index.get_value().clone();
                            self.rewrite_single(&mut value, preambles);
                            if spillable
                                && later_may_inline
                                && clean
                                && self.position_allows_insertion()
                            {
                                value = self.materialize(value, preambles);
                                self.clean = true;
                            }
                            *index.mutate_value() = value;
                            replacement.push(crate::nodes::TableEntry::Index(index));
                        }
                        crate::nodes::TableEntry::Value(value) => {
                            // The last array entry takes every value, like any
                            // other trailing value list.
                            let results = self.rewrite((*value).clone(), tail, preambles);
                            for result in results {
                                let result = if later_may_inline
                                    && clean
                                    && self.position_allows_insertion()
                                {
                                    self.clean = true;
                                    self.materialize(result, preambles)
                                } else {
                                    result
                                };
                                replacement.push(crate::nodes::TableEntry::Value(Box::new(result)));
                            }
                        }
                    }
                    if !spillable {
                        // Entries after a computed key cannot be hoisted.
                        self.clean = false;
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
            Prefix::Identifier(identifier) => {
                // Reading the callee of an inlinable local function is stable:
                // the symbol is never written after it is bound.
                if !self.callee_is_inlinable(identifier) {
                    self.clean = false;
                }
            }
            Prefix::Field(field) => {
                let mut inner = field.get_prefix().clone();
                self.rewrite_prefix(&mut inner, preambles);
                *field.mutate_prefix() = inner;
                // A field read can trigger `__index`.
                self.clean = false;
            }
            Prefix::Index(index) => {
                let mut inner = index.get_prefix().clone();
                self.rewrite_prefix(&mut inner, preambles);
                *index.mutate_prefix() = inner;
                let mut key = index.get_index().clone();
                self.rewrite_single(&mut key, preambles);
                *index.mutate_index() = key;
                // An index read can trigger `__index`.
                self.clean = false;
            }
            Prefix::Call(call) => {
                let mut inner = call.as_ref().clone();
                // A call used as a prefix consumes one value.
                if let Some(mut replacement) = self.try_inline(&inner, false, preambles) {
                    let value = replacement.remove(0);
                    *prefix = Prefix::Parenthese(Box::new(
                        crate::nodes::ParentheseExpression::new(value),
                    ));
                } else {
                    self.rewrite_call_contents(&mut inner, preambles);
                    self.clean = false;
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
        if !self.clean || call.get_method().is_some() {
            return None;
        }
        // Inserting the preamble declares locals before the call's statement. A
        // label in the same function could be the target of a goto that jumps
        // over them, which would be invalid once the locals exist.
        let position = self.current_position?;
        if !self.position_allows_insertion() {
            return None;
        }
        let callee = self.model.resolve_identifier(callee(call.get_prefix())?)?;
        let (symbol, hops) = self.resolve_function(callee)?;
        let function = self.functions.get(&symbol)?;
        if tail && !function.uniform {
            return None;
        }

        // Each hop captures the value it reads, so the hop must run after that
        // value is bound and before the position that reads the hop.
        let mut reader = position;
        for hop in hops {
            if !self.assignment_reaches(hop, reader) {
                return None;
            }
            reader = hop;
        }
        if let Some(assignment) = function.assignment {
            if !self.assignment_reaches(assignment, reader) {
                return None;
            }
        }

        let scope = self.current_scope?;
        for upvalue in &function.upvalues {
            if !self.upvalue_is_reachable(*upvalue, scope) {
                return None;
            }
            // A global access resolves through `_ENV`; if the call site
            // shadows it, the inlined access would use a different environment.
            if self.model.symbol(*upvalue).kind == SymbolKind::Global
                && !self.environment_is_reachable(scope)
            {
                return None;
            }
        }

        let results: Vec<String> = (0..function.arity).map(|_| self.names.generate()).collect();
        let done = self.names.generate();

        // The arguments are part of the call being replaced, so their own calls
        // can be inlined first; their preambles run before this body.
        let mut values = call.get_arguments().clone().to_expressions();
        self.rewrite_values(&mut values, true, preambles);

        let mut bindings: Vec<TypedIdentifier> = function
            .parameters
            .iter()
            .map(|name| TypedIdentifier::new(name.clone()))
            .collect();
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

    /// Resolves a call target through immutable local aliases to a recorded
    /// function.
    ///
    /// Returns the function's symbol and the declaration positions of the
    /// aliases on the way, so an assignment-bound function can prove it was
    /// the value each alias captured.
    fn resolve_function(&self, mut symbol: SymbolId) -> Option<(SymbolId, Vec<StatementOrigin>)> {
        let mut hops = Vec::new();
        for _ in 0..self.model.symbols.len() {
            if self.functions.contains_key(&symbol) {
                return Some((symbol, hops));
            }
            let next = if let Some((target, position)) = self.aliases.get(&symbol) {
                hops.push(*position);
                *target
            } else if self.model.is_immutable(symbol) {
                let next = self.model.symbol(symbol).initializer?;
                hops.push(*self.declarations.get(&symbol)?);
                next
            } else {
                return None;
            };
            if next == symbol {
                return None;
            }
            symbol = next;
        }
        None
    }

    /// Returns whether the assignment at `origin` runs before `position` on
    /// every path, reasoning only about structured control flow.
    ///
    /// The position's enclosing blocks form a chain. The assignment must be on
    /// that chain with the statement leading toward the position after it. An
    /// assignment deeper than the position is lifted out of each enclosing `do`
    /// body, because a `do` body always runs to completion before its statement
    /// finishes.
    ///
    /// A call inside a nested function is accepted when the assignment precedes
    /// the function's definition, because the body cannot run before the
    /// definition completes.
    fn assignment_reaches(&self, assignment: StatementOrigin, position: StatementOrigin) -> bool {
        let Some(function) = self.block_functions.get(&position.block) else {
            return false;
        };
        if self.labelled.contains(function) {
            return false;
        }

        let mut chain = vec![position];
        let mut current = position;
        let mut remaining = self.parents.len() + 1;
        while let Some(&(parent, index)) = self.parents.get(&current.block) {
            if parent == current.block || remaining == 0 {
                return false;
            }
            current = StatementOrigin {
                block: parent,
                index,
            };
            chain.push(current);
            remaining -= 1;
        }

        let mut current = assignment;
        loop {
            if let Some(entry) = chain.iter().find(|entry| entry.block == current.block) {
                return current.index < entry.index;
            }
            if !self.unconditional.contains(&current.block) {
                return false;
            }
            let Some(&(parent, index)) = self.parents.get(&current.block) else {
                return false;
            };
            if parent == current.block {
                return false;
            }
            current = StatementOrigin {
                block: parent,
                index,
            };
        }
    }

    fn table_entry_may_inline(&self, entry: &crate::nodes::TableEntry) -> bool {
        match entry {
            crate::nodes::TableEntry::Field(field) => self.expression_may_inline(field.get_value()),
            crate::nodes::TableEntry::Index(index) => self.expression_may_inline(index.get_value()),
            crate::nodes::TableEntry::Value(value) => self.expression_may_inline(value),
        }
    }

    /// Returns whether `expression` is a bare identifier that the VM reads from
    /// its register when the surrounding instruction runs, rather than eagerly
    /// into a temporary.
    fn is_delayed_operand(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Identifier(identifier) => self.is_current_local(identifier),
            _ => false,
        }
    }

    fn prefix_is_delayed(&self, prefix: &Prefix) -> bool {
        match prefix {
            Prefix::Identifier(identifier) => self.is_current_local(identifier),
            _ => false,
        }
    }

    /// Returns whether `identifier` names a local declared in the function that
    /// contains the current statement. Upvalues and globals are loaded into
    /// temporaries before a later operand runs.
    fn is_current_local(&self, identifier: &Identifier) -> bool {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return false;
        };
        let info = self.model.symbol(symbol);
        if info.kind != SymbolKind::Local {
            return false;
        }
        let Some(position) = self.current_position else {
            return false;
        };
        let Some(function) = self.block_functions.get(&position.block) else {
            return false;
        };
        let mut scope = Some(info.declaring_scope);
        while let Some(id) = scope {
            if id == *function {
                return true;
            }
            scope = self.model.scope(id).parent;
        }
        false
    }

    /// Returns whether `expression` contains a call that could be inlined.
    ///
    /// This over-approximates: it ignores dominance, shadowing, arity, and
    /// conditional positions. A false positive only stores an earlier value in
    /// a temporary.
    fn expression_may_inline(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Call(call) => self.call_may_inline(call),
            Expression::Binary(binary) => {
                self.expression_may_inline(binary.left())
                    || self.expression_may_inline(binary.right())
            }
            Expression::Unary(unary) => self.expression_may_inline(unary.get_expression()),
            Expression::Parenthese(parenthese) => {
                self.expression_may_inline(parenthese.inner_expression())
            }
            Expression::TypeCast(type_cast) => {
                self.expression_may_inline(type_cast.get_expression())
            }
            Expression::TypeInstantiation(instantiation) => {
                self.prefix_may_inline(instantiation.get_prefix())
            }
            Expression::Field(field) => self.prefix_may_inline(field.get_prefix()),
            Expression::Index(index) => {
                self.prefix_may_inline(index.get_prefix())
                    || self.expression_may_inline(index.get_index())
            }
            Expression::Table(table) => table.iter_entries().any(|entry| match entry {
                crate::nodes::TableEntry::Field(field) => {
                    self.expression_may_inline(field.get_value())
                }
                crate::nodes::TableEntry::Index(index) => {
                    self.expression_may_inline(index.get_key())
                        || self.expression_may_inline(index.get_value())
                }
                crate::nodes::TableEntry::Value(value) => self.expression_may_inline(value),
            }),
            Expression::If(if_expression) => {
                self.expression_may_inline(if_expression.get_condition())
                    || self.expression_may_inline(if_expression.get_result())
                    || if_expression.iter_branches().any(|branch| {
                        self.expression_may_inline(branch.get_condition())
                            || self.expression_may_inline(branch.get_result())
                    })
                    || self.expression_may_inline(if_expression.get_else_result())
            }
            Expression::InterpolatedString(string) => {
                string.iter_segments().any(|segment| match segment {
                    crate::nodes::InterpolationSegment::Value(value) => {
                        self.expression_may_inline(value.get_expression())
                    }
                    _ => false,
                })
            }
            _ => false,
        }
    }

    fn prefix_may_inline(&self, prefix: &Prefix) -> bool {
        match prefix {
            Prefix::Identifier(_) => false,
            Prefix::Call(call) => self.call_may_inline(call),
            Prefix::Field(field) => self.prefix_may_inline(field.get_prefix()),
            Prefix::Index(index) => {
                self.prefix_may_inline(index.get_prefix())
                    || self.expression_may_inline(index.get_index())
            }
            Prefix::Parenthese(parenthese) => {
                self.expression_may_inline(parenthese.inner_expression())
            }
            Prefix::TypeInstantiation(instantiation) => {
                self.prefix_may_inline(instantiation.get_prefix())
            }
        }
    }

    fn call_may_inline(&self, call: &crate::nodes::FunctionCall) -> bool {
        if self.prefix_may_inline(call.get_prefix())
            || call
                .get_arguments()
                .clone()
                .to_expressions()
                .iter()
                .any(|argument| self.expression_may_inline(argument))
        {
            return true;
        }
        if call.get_method().is_some() {
            return false;
        }
        let Some(identifier) = callee(call.get_prefix()) else {
            return false;
        };
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return false;
        };
        self.resolve_function(symbol).is_some()
    }

    /// Returns whether reading `identifier` as a callee is stable: it resolves
    /// to a recorded local function whose binding is never replaced.
    fn callee_is_inlinable(&self, identifier: &Identifier) -> bool {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return false;
        };
        self.resolve_function(symbol).is_some()
    }

    /// Returns whether no scope between `scope` and the global scope declares an
    /// explicit `_ENV`.
    fn environment_is_reachable(&self, mut scope: ScopeId) -> bool {
        loop {
            if self
                .model
                .scope(scope)
                .symbols
                .iter()
                .any(|symbol| self.model.symbol(*symbol).name == "_ENV")
            {
                return false;
            }
            let Some(parent) = self.model.scope(scope).parent else {
                return true;
            };
            scope = parent;
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

/// Returns the identifier a call's prefix names, looking through parentheses
/// and type instantiation.
/// Returns whether a table key is a constant that is evaluated without
/// reading a variable or running code.
fn expression_is_literal_key(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Number(_) | Expression::String(_) | Expression::True(_) | Expression::False(_)
    )
}

/// Returns whether evaluating `expression` is unobservable and cannot raise.
fn expression_is_inert(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Nil(_)
            | Expression::True(_)
            | Expression::False(_)
            | Expression::Number(_)
            | Expression::String(_)
    )
}

fn prefix_into_expression(prefix: Prefix) -> Expression {
    match prefix {
        Prefix::Call(call) => Expression::Call(call),
        Prefix::Field(field) => Expression::Field(field),
        Prefix::Identifier(identifier) => Expression::Identifier(identifier),
        Prefix::Index(index) => Expression::Index(index),
        Prefix::Parenthese(parenthese) => Expression::Parenthese(parenthese),
        Prefix::TypeInstantiation(instantiation) => Expression::TypeInstantiation(instantiation),
    }
}

fn callee(prefix: &Prefix) -> Option<&Identifier> {
    match prefix {
        Prefix::Identifier(identifier) => Some(identifier),
        Prefix::Parenthese(parenthese) => callee_expression(parenthese.inner_expression()),
        Prefix::TypeInstantiation(instantiation) => callee(instantiation.get_prefix()),
        _ => None,
    }
}

/// Unwraps the expression forms that preserve a callee's identity: parentheses
/// and type assertions.
fn callee_expression(expression: &Expression) -> Option<&Identifier> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier),
        Expression::Parenthese(parenthese) => callee_expression(parenthese.inner_expression()),
        Expression::TypeCast(type_cast) => callee_expression(type_cast.get_expression()),
        Expression::TypeInstantiation(instantiation) => callee(instantiation.get_prefix()),
        _ => None,
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
        inline_local_functions(&mut block, &model).expect("the analyzed tree has block identities");
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
    fn inlines_a_variable_arity_return_in_a_single_value_position() {
        let output = inlined_output(
            "local function first()\n    return pick()\nend\nlocal x = first()\nreturn x",
        );
        assert!(!output.contains("= first()"), "{output}");
    }

    #[test]
    fn inlines_a_parenthesized_callee() {
        let output = inlined_output("local function f(a)\n    return a\nend\nreturn (f)(1)");
        assert!(!output.contains("(f)(1)"), "{output}");
    }

    #[test]
    fn inlines_a_doubly_parenthesized_callee() {
        let output = inlined_output("local function f(a)\n    return a\nend\nreturn ((f))(1)");
        assert!(!output.contains("((f))(1)"), "{output}");
    }

    #[test]
    fn inlines_a_type_cast_callee() {
        let (mut block, model) = crate::rules::test_support::analyze_luau_with_tokens(
            "local function f(a)\n    return a\nend\nreturn (f :: (number) -> number)(1)",
        );
        inline_local_functions(&mut block, &model).expect("the analyzed tree has block identities");
        let output = generate(&block);
        assert!(!output.contains("(f :: (number) -> number)(1)"), "{output}");
    }

    #[test]
    fn inlines_through_an_immutable_alias() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal sum = add\nreturn sum(1, 2)",
        );
        assert!(!output.contains("sum(1, 2)"), "{output}");
    }

    #[test]
    fn inlines_a_function_bound_by_an_assignment() {
        let output = inlined_output("local f\nf = function(x)\n    return x + 1\nend\nreturn f(1)");
        assert!(!output.contains("f(1)"), "{output}");
        assert!(output.contains("return x + 1"), "{output}");
    }

    #[test]
    fn inlines_an_assignment_bound_call_in_a_nested_block() {
        let output = inlined_output(
            "local f\nf = function()\n    return 1\nend\nif true then\n    return f()\nend",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_an_assignment_inside_an_unconditional_block() {
        let output = inlined_output(
            "local f\ndo\n    f = function()\n        return 1\n    end\nend\nreturn f()",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_an_assignment_inside_nested_unconditional_blocks() {
        let output = inlined_output(
            "local f\ndo\n    do\n        f = function()\n            return 1\n        end\n    end\n    local value = f()\nend\nreturn 0",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn keeps_an_assignment_inside_a_repeat_body() {
        let output = inlined_output(
            "local f\nrepeat\n    f = function()\n        return 1\n    end\nuntil true\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_an_assignment_bound_call_after_an_unconditional_block() {
        let output = inlined_output(
            "local f\ndo\n    f = function()\n        return 1\n    end\n    return f()\nend",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_an_assignment_bound_function_through_an_alias_declared_after_it() {
        let output =
            inlined_output("local f\nf = function(x)\n    return x\nend\nlocal g = f\nreturn g(1)");
        assert!(!output.contains("g(1)"), "{output}");
    }

    #[test]
    fn inlines_through_an_assigned_alias() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal sum\nsum = add\nreturn sum(1, 2)",
        );
        assert!(!output.contains("sum(1, 2)"), "{output}");
    }

    #[test]
    fn keeps_an_assigned_alias_under_a_conditional() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal sum\nif true then\n    sum = add\nend\nreturn sum(1, 2)",
        );
        assert!(output.contains("sum(1, 2)"), "{output}");
    }

    #[test]
    fn keeps_an_assigned_alias_read_before_the_assignment() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal sum\nlocal function call()\n    return sum(1, 2)\nend\nsum = add\nreturn call()",
        );
        assert!(output.contains("sum(1, 2)"), "{output}");
    }

    #[test]
    fn keeps_an_alias_chain_in_the_wrong_order() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal first\nlocal second\nfirst = second\nsecond = add\nreturn first(1, 2)",
        );
        assert!(output.contains("first(1, 2)"), "{output}");
    }

    #[test]
    fn keeps_an_alias_declared_before_the_assignment() {
        let output =
            inlined_output("local f\nlocal g = f\nf = function()\n    return 1\nend\nreturn g()");
        assert!(output.contains("g()"), "{output}");
    }

    #[test]
    fn inlines_inside_a_function_expression_defined_after_the_assignment() {
        let output = inlined_output(
            "local f\nf = function(x)\n    return x\nend\nlocal g = function()\n    return f(1)\nend\nreturn g()",
        );
        assert!(!output.contains("f(1)"), "{output}");
    }

    #[test]
    fn keeps_a_call_inside_a_function_expression_defined_before_the_assignment() {
        let output = inlined_output(
            "local f\nlocal g = function()\n    return f(1)\nend\nf = function(x)\n    return x\nend\nreturn g()",
        );
        assert!(output.contains("f(1)"), "{output}");
    }

    #[test]
    fn keeps_a_call_that_depends_on_another_function_running_the_assignment() {
        let output = inlined_output(
            "local f\nlocal function set()\n    f = function()\n        return 1\n    end\nend\nset()\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_a_body_that_writes_an_upvalue() {
        let output = inlined_output(
            "local value = 0\nlocal function set(x)\n    value = x\nend\nset(5)\nreturn value",
        );
        assert!(!output.contains("set(5)"), "{output}");
        assert!(output.contains("value = x"), "{output}");
    }

    #[test]
    fn keeps_a_write_to_an_upvalue_shadowed_at_the_call_site() {
        let output = inlined_output(
            "local value = 0\nlocal function set()\n    value = 5\nend\ndo\n    local value = 100\n    set()\nend\nreturn value",
        );
        assert!(output.contains("set()"), "{output}");
    }

    #[test]
    fn keeps_a_prefix_read_of_an_upvalue_shadowed_at_the_call_site() {
        let output = inlined_output(
            "local box = { value = 1 }\nlocal function read()\n    return box.value\nend\ndo\n    local box = { value = 2 }\n    return read()\nend",
        );
        assert!(output.contains("read()"), "{output}");
    }

    #[test]
    fn keeps_a_global_read_when_the_environment_is_shadowed() {
        let output = inlined_output(
            "local function show()\n    return tostring(1)\nend\ndo\n    local _ENV = {}\n    return show()\nend",
        );
        assert!(output.contains("show()"), "{output}");
    }

    #[test]
    fn keeps_a_global_shadowed_at_the_call_site() {
        let output = inlined_output(
            "local function hello()\n    print(1)\nend\ndo\n    local print = function() end\n    hello()\nend\nreturn 0",
        );
        assert!(output.contains("hello()"), "{output}");
    }

    #[test]
    fn keeps_a_nested_return_upvalue_shadowed_at_the_call_site() {
        let output = inlined_output(
            "local value = 1\nlocal function make()\n    return function()\n        return value\n    end\nend\ndo\n    local value = 2\n    return make()\nend",
        );
        assert!(output.contains("make()"), "{output}");
    }

    #[test]
    fn inlines_a_call_argument_after_another_argument() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal first = 1\nprint(first, add(1, 2))\nreturn 0",
        );
        assert!(!output.contains("add(1, 2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_argument_after_another_call() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal function one()\n    return 1\nend\nprint(one(), add(1, 2))\nreturn 0",
        );
        assert!(!output.contains("add(1, 2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_in_a_return_list() {
        let output = inlined_output(
            "local function add(a, b)\n    return a + b\nend\nlocal first = 1\nreturn first, add(1, 2)",
        );
        assert!(!output.contains("add(1, 2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_in_a_binary_operand() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal first = 1\nreturn first + value(2)",
        );
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_in_a_table_entry_after_a_value() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal first = 1\nlocal t = { first, value(2) }\nreturn t",
        );
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_in_a_keyed_table_entry() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal first = 1\nlocal t = { item = first, other = value(2) }\nreturn t",
        );
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn keeps_a_call_after_a_computed_table_key() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal key = \"item\"\nlocal t = { [key] = 1, other = value(2) }\nreturn t",
        );
        assert!(output.contains("value(2)"), "{output}");
    }

    #[test]
    fn inlines_a_nested_call_argument() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal function wrap(x)\n    return x\nend\nreturn wrap(value(2))",
        );
        assert!(!output.contains("wrap(value(2))"), "{output}");
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_in_an_index_key() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal t = { 1, 2 }\nreturn t[value(2)]",
        );
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn keeps_a_method_argument_when_the_receiver_precedes_it() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal object = { method = function(self, x) return x end }\nreturn object:method(value(2))",
        );
        assert!(output.contains("value(2)"), "{output}");
    }

    #[test]
    fn keeps_a_conditional_operand_after_a_read() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal first = 1\nreturn first and value(2)",
        );
        assert!(output.contains("value(2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_after_a_literal() {
        let output =
            inlined_output("local function value(x)\n    return x\nend\nreturn 1, value(2)");
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn inlines_a_call_after_an_evaluated_expression() {
        let output = inlined_output(
            "local function value(x)\n    return x\nend\nlocal first = 1\nreturn first, value(2)",
        );
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn keeps_a_call_in_a_short_circuit_operand() {
        let output =
            inlined_output("local function value(x)\n    return x\nend\nreturn false and value(2)");
        assert!(output.contains("value(2)"), "{output}");
    }

    #[test]
    fn keeps_a_call_in_a_while_condition() {
        let output = inlined_output(
            "local function check(x)\n    return x\nend\nwhile check(true) do\n    break\nend\nreturn 0",
        );
        assert!(output.contains("check(true)"), "{output}");
    }

    #[test]
    fn keeps_a_call_in_a_repeat_condition() {
        let output = inlined_output(
            "local function check(x)\n    return x\nend\nrepeat\n    break\nuntil check(true)\nreturn 0",
        );
        assert!(output.contains("check(true)"), "{output}");
    }

    #[test]
    fn keeps_a_call_in_an_elseif_condition() {
        let output = inlined_output(
            "local function check(x)\n    return x\nend\nif true then\n    return 0\nelseif check(true) then\n    return 1\nend\nreturn 2",
        );
        assert!(output.contains("check(true)"), "{output}");
    }

    #[test]
    fn inlines_a_call_in_the_argument_of_another_call() {
        let output =
            inlined_output("local function value(x)\n    return x\nend\nprint(value(2))\nreturn 0");
        assert!(!output.contains("value(2)"), "{output}");
    }

    #[test]
    fn keeps_a_call_before_the_assignment() {
        let output = inlined_output(
            "local f\nlocal value = f\nf = function()\n    return 1\nend\nreturn value",
        );
        assert!(output.contains("local value = f"), "{output}");
    }

    #[test]
    fn keeps_a_call_under_a_conditional_assignment() {
        let output = inlined_output(
            "local f\nif true then\n    f = function()\n        return 1\n    end\nend\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn keeps_an_assignment_bound_call_after_a_goto() {
        let output = inlined_output(
            "local f\nf = function()\n    return 1\nend\ngoto done\n::done::\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
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
            "local function first()\n    return pick()\nend\nlocal x = first()\nreturn x",
            "local function f(a)\n    return a\nend\nreturn (f)(1)",
            "local function add(a, b)\n    return a + b\nend\nlocal sum = add\nreturn sum(1, 2)",
            "local f\nf = function(x)\n    return x + 1\nend\nreturn f(1)",
            "local f\nf = function(x)\n    return x\nend\nlocal g = f\nreturn g(1)",
            "local f\nlocal g = f\nf = function()\n    return 1\nend\nreturn g()",
            "local f\nf = function(x)\n    return x\nend\nlocal g = function()\n    return f(1)\nend\nreturn g()",
            "local f\nlocal g = function()\n    return f(1)\nend\nf = function(x)\n    return x\nend\nreturn g()",
            "local f\ndo\n    f = function()\n        return 1\n    end\nend\nreturn f()",
            "local f\nrepeat\n    f = function()\n        return 1\n    end\nuntil true\nreturn f()",
            "local function check(x)\n    return x\nend\nwhile check(true) do\n    break\nend\nreturn 0",
            "local function check(x)\n    return x\nend\nrepeat\n    break\nuntil check(true)\nreturn 0",
            "local function value(x)\n    return x\nend\nreturn false and value(2)",
            "local function add(a, b)\n    return a + b\nend\nlocal first = 1\nprint(first, add(1, 2))\nreturn 0",
            "local function add(a, b)\n    return a + b\nend\nlocal first = 1\nreturn first, add(1, 2)",
            "local function value(x)\n    return x\nend\nlocal first = 1\nreturn first + value(2)",
            "local function value(x)\n    return x\nend\nlocal t = { 1, 2 }\nreturn t[value(2)]",
            "local function value(x)\n    return x\nend\nlocal function wrap(x)\n    return x\nend\nreturn wrap(value(2))",
            "local function value(x)\n    return x\nend\nlocal first = 1\nlocal t = { first, value(2) }\nreturn t",
            "local function value(x)\n    return x\nend\nlocal first = 1\nlocal t = { item = first, other = value(2) }\nreturn t",
            "local function value(x)\n    return x\nend\nlocal first = 1\nlocal t = { [first] = 1, other = value(2) }\nreturn t",
            "local function add(a, b)\n    return a + b\nend\nlocal sum\nsum = add\nreturn sum(1, 2)",
            "local function add(a, b)\n    return a + b\nend\nlocal first\nlocal second\nfirst = second\nsecond = add\nreturn first(1, 2)",
            "local value = 0\nlocal function set(x)\n    value = x\nend\nset(5)\nreturn value",
            "local value = 0\nlocal function set()\n    value = 5\nend\ndo\n    local value = 100\n    set()\nend\nreturn value",
        ] {
            let output = inlined_output(code);
            parse(&output);
        }
    }
}
