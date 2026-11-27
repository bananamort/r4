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
//! Arguments bind to parameters exactly as the call did, so each is evaluated
//! once, in order, with the same multi-value adjustment. Earlier list values,
//! table entries, and eagerly-read operands are stored in fresh locals in their
//! original order when a later argument can inline. A bare local of the current
//! function is read by the binary operator, index, or method call itself after
//! the other operand runs, so it is left in place; concatenation operands and
//! globals, upvalues, and compound expressions are read eagerly and are stored.
//!
//! # Which binding a call sees
//!
//! The callee is resolved through the reaching definitions of its function
//! region, so conditional assignments, loops, `goto` paths, and aliases are
//! handled exactly:
//!
//! * every reaching definition must denote a function whose parameters and
//!   body are identical; otherwise a later call could observe a different
//!   value and the call is left alone;
//! * an alias resolves at the statement where the alias captured its value,
//!   not at the call;
//! * a call inside a nested function reads an upvalue. An immutable upvalue is
//!   fixed. A mutable one is usable when no write to it can run after the
//!   creation of the outermost closure in the chain that captures it. A write
//!   after that creation could reach a later call, so that shape is refused.
//!
//! # Method calls
//!
//! When an argument may inline, the receiver is stored before the argument
//! bodies. PUC Lua and LuaJIT perform the method lookup before the arguments,
//! so the lookup value is stored there too and the call becomes a normal call
//! with the receiver as its first argument. Luau evaluates the arguments before
//! `NAMECALL`, so it keeps the method syntax with the stored receiver and the
//! lookup stays at the call. An undetermined dialect only reorders when the
//! receiver is a fresh table literal, whose lookup cannot run code.
//!
//! # Loop conditions
//!
//! A `while` condition that inlines becomes `while true do <preamble>;
//! if not <condition> then break end; <body> end`, so the condition still runs
//! before every iteration. `continue` already jumps to the loop condition.
//!
//! A `repeat` condition stays at the end of the body, where the body's
//! top-level locals are visible. The inline preamble is appended there and the
//! loop becomes `until <result>`. Because `continue` skips the rest of the body
//! but not the condition, each `continue` is lowered to a flag assignment and
//! the statements after it are guarded with `if not <flag> then ... end`; the
//! condition section sits after those guards and always runs. If a guard would
//! move a condition-visible local into a nested scope, its declaration is moved
//! to the top of the loop body under a fresh name and every bound reference is
//! renamed, preserving the per-iteration cell for closures. A repeat condition
//! that is unreachable after a final `return`/`break`, or whose `continue`
//! lowering would move a `<close>` local's closing point, is left alone.
//!
//! # `goto` and labels
//!
//! Labels and gotos in the callee body are renamed with fresh names. In a
//! function that contains a `goto` or label, locals that escape the preamble
//! are declared at the start of the function instead of before the call, so a
//! forward jump never enters their scope. The inline body itself is wrapped in
//! a `do` block, whose locals never escape. Label names are reserved by the
//! name generator, so two inlined copies and the caller cannot collide.
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
//! Language limits remain: a body that reads `...` cannot be reproduced
//! because `...` is lexical to a function; a body whose value count depends on
//! a call is not usable in a value-consuming slot; and a name that is shadowed
//! at the call site cannot be reproduced. These are refused rather than
//! rewritten into different behavior.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::cfg::{
    build_chunk, build_function, validate_block_identities, CfgError, ConditionKind,
    ConditionOrigin, StatementOrigin, UnidentifiedBlock,
};
use crate::nodes::{
    Arguments, AssignStatement, BinaryOperator, Block, BlockIdentity, ChildVisitor, DoStatement,
    Expression, Identifier, LastStatement, Prefix, ReadOnlyChildren, RepeatStatement, Statement,
    TableExpression, TypedIdentifier, UnaryExpression, UnaryOperator, Variable, VariableAssignment,
    WhileStatement,
};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::rules::NameGenerator;
use crate::semantic::{
    LabelId, ScopeId, SemanticModel, SemanticQueryError, StatementPosition, SymbolId, SymbolKind,
};

mod reaching;

use reaching::{DefinitionValue, FunctionRegion, ReachingError};

/// Identifies one function literal by the statement that assigns it and the
/// variable slot it occupies in that statement.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct FunctionKey {
    origin: StatementOrigin,
    slot: u32,
}

/// A local function that can be inlined at its call sites.
#[derive(Clone, Debug)]
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
}

impl InlinableFunction {
    /// Returns whether two definitions denote behaviorally identical functions.
    fn same_function(&self, other: &Self) -> bool {
        self.parameters == other.parameters
            && self.body == other.body
            && self.arity == other.arity
            && self.uniform == other.uniform
            && self.upvalues == other.upvalues
    }
}

/// An error produced while inlining local functions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineError {
    /// A block has no structural identity, so statement positions cannot be
    /// trusted.
    Provenance(UnidentifiedBlock),
    /// The semantic model and the tree disagree on a statement or identifier,
    /// or a function's control flow could not be built.
    Reaching(ReachingError),
    /// A block could not be mapped to the analyzed scope it belongs to.
    Scope(SemanticQueryError),
}

impl fmt::Display for InlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provenance(error) => write!(f, "cannot inline local functions: {error}"),
            Self::Reaching(error) => write!(f, "cannot inline local functions: {error}"),
            Self::Scope(error) => write!(f, "cannot inline local functions: {error}"),
        }
    }
}

impl std::error::Error for InlineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Provenance(error) => Some(error),
            Self::Reaching(error) => Some(error),
            Self::Scope(error) => Some(error),
        }
    }
}

impl From<UnidentifiedBlock> for InlineError {
    fn from(error: UnidentifiedBlock) -> Self {
        Self::Provenance(error)
    }
}

impl From<ReachingError> for InlineError {
    fn from(error: ReachingError) -> Self {
        Self::Reaching(error)
    }
}

impl From<CfgError> for InlineError {
    fn from(error: CfgError) -> Self {
        Self::Reaching(ReachingError::ControlFlow(error))
    }
}

impl From<SemanticQueryError> for InlineError {
    fn from(error: SemanticQueryError) -> Self {
        Self::Scope(error)
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

    let collector = FunctionCollector::collect(block, model)?;

    if collector.functions.is_empty() {
        return Ok(0);
    }

    let mut names = NameGenerator::new(model);
    let mut inlined = 0;
    let mut inliner = Inliner {
        model,
        functions: &collector.functions,
        literal_symbols: &collector.literal_symbols,
        aliases: &collector.aliases,
        parents: &collector.parents,
        function_roots: &collector.function_roots,
        labelled: &collector.labelled,
        block_functions: &collector.block_functions,
        regions: &collector.regions,
        definition_locations: &collector.definition_locations,
        nested_writes: &collector.nested_writes,
        names: &mut names,
        current_scope: None,
        current_position: None,
        current_condition: None,
        clean: false,
        allow_inline: true,
        hoist_locals: true,
        hoisted: HashMap::new(),
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
) -> Option<InlinableFunction> {
    let info = model.symbol(symbol);
    if info.kind != SymbolKind::Local {
        return None;
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
/// assignment, the enclosing statement of every block, the reaching
/// definitions of every function region, and the function scopes that contain a
/// `goto` or label.
struct FunctionCollector<'m> {
    model: &'m SemanticModel,
    /// Every function literal assigned to a local, keyed by its structural
    /// position. `None` records a literal that the inliner cannot reproduce.
    functions: HashMap<FunctionKey, Option<InlinableFunction>>,
    /// Local symbols that receive at least one function literal. This is an
    /// over-approximation used only to decide when an earlier value must be
    /// stored.
    literal_symbols: HashSet<SymbolId>,
    /// Direct `alias = target` assignments, for the same over-approximation.
    aliases: HashMap<SymbolId, HashSet<SymbolId>>,
    /// The statement that contains each AST block.
    parents: HashMap<BlockIdentity, (BlockIdentity, u32)>,
    /// The body block of each function region.
    function_roots: HashMap<ScopeId, BlockIdentity>,
    /// Function scopes whose bodies contain a `goto` or label.
    labelled: HashSet<ScopeId>,
    /// The function scope that owns each block.
    block_functions: HashMap<BlockIdentity, ScopeId>,
    /// Reaching definitions for each function region.
    regions: HashMap<ScopeId, FunctionRegion>,
    /// Where each symbol is defined.
    definition_locations: HashMap<SymbolId, Vec<(ScopeId, reaching::DefinitionId)>>,
    /// Captured locals written by a nested function. Such a local can change
    /// whenever that function runs, so only its single immutable definition is
    /// usable.
    nested_writes: HashSet<SymbolId>,
    /// The first structural inconsistency, if any.
    error: Option<InlineError>,
}

impl<'m> FunctionCollector<'m> {
    fn collect(root: &Block, model: &'m SemanticModel) -> Result<Self, InlineError> {
        let mut collector = Self {
            model,
            functions: HashMap::new(),
            literal_symbols: HashSet::new(),
            aliases: HashMap::new(),
            parents: HashMap::new(),
            function_roots: HashMap::new(),
            labelled: HashSet::new(),
            block_functions: HashMap::new(),
            regions: HashMap::new(),
            definition_locations: HashMap::new(),
            nested_writes: nested_writes(model),
            error: None,
        };
        collector.walk(root);
        if let Some(error) = collector.error {
            return Err(error);
        }

        let mut locations: HashMap<SymbolId, Vec<(ScopeId, reaching::DefinitionId)>> =
            HashMap::new();
        for (scope, region) in &collector.regions {
            for (id, definition) in region.definitions.iter().enumerate() {
                locations
                    .entry(definition.symbol)
                    .or_default()
                    .push((*scope, id));
            }
        }
        collector.definition_locations = locations;
        Ok(collector)
    }

    fn walk(&mut self, block: &Block) {
        if self.error.is_some() {
            return;
        }
        let Some(identity) = block.get_identity() else {
            return;
        };
        self.ensure_region(block);
        if self.error.is_some() {
            return;
        }
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
            if self.error.is_some() {
                return;
            }
            if matches!(statement, Statement::Goto(_) | Statement::Label(_)) {
                if let Some(function) = function {
                    self.labelled.insert(function);
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

    /// Builds the control-flow graph and reaching definitions of the function
    /// region that owns `block`, the first time the block is visited.
    fn ensure_region(&mut self, block: &Block) {
        let Some(identity) = block.get_identity() else {
            return;
        };
        let Ok(scope) = self.model.try_scope_of(block) else {
            let error = self
                .model
                .try_scope_of(block)
                .expect_err("the scope query just failed");
            self.error = Some(InlineError::Scope(error));
            return;
        };
        if !self.model.scope(scope).kind.is_function_boundary() || self.regions.contains_key(&scope)
        {
            return;
        }
        let cfg = if scope == self.model.chunk_scope() {
            build_chunk(block, self.model)
        } else {
            build_function(block, self.model, scope)
        };
        match cfg {
            Ok(cfg) => match FunctionRegion::analyze(cfg, self.model) {
                Ok(region) => {
                    self.regions.insert(scope, region);
                    self.function_roots.insert(scope, identity);
                }
                Err(error) => self.error = Some(error.into()),
            },
            Err(error) => self.error = Some(error.into()),
        }
    }

    fn record_statement(&mut self, statement: &Statement, origin: StatementOrigin) {
        match statement {
            Statement::LocalFunction(function) => {
                let parameters: Vec<&TypedIdentifier> = function.iter_parameters().collect();
                self.record_literal(
                    FunctionKey { origin, slot: 0 },
                    function.get_identifier(),
                    function.get_block(),
                    &parameters,
                );
            }
            Statement::Function(function) => {
                let name = function.get_name();
                if !name.get_field_names().is_empty() || name.get_method().is_some() {
                    return;
                }
                let parameters: Vec<&TypedIdentifier> = function.iter_parameters().collect();
                self.record_literal(
                    FunctionKey { origin, slot: 0 },
                    name.get_name(),
                    function.get_block(),
                    &parameters,
                );
            }
            Statement::LocalAssign(assignment) => {
                let values: Vec<&Expression> = assignment.iter_values().collect();
                for (index, variable) in assignment.iter_variables().enumerate() {
                    let Some(value) = values.get(index) else {
                        continue;
                    };
                    self.record_initializer(variable.get_identifier(), value, origin, index);
                }
            }
            Statement::Assign(assignment) => {
                let values: Vec<&Expression> = assignment.iter_values().collect();
                for (index, variable) in assignment.iter_variables().enumerate() {
                    let Variable::Identifier(identifier) = variable else {
                        continue;
                    };
                    let Some(value) = values.get(index) else {
                        continue;
                    };
                    self.record_initializer(identifier, value, origin, index);
                }
            }
            _ => {}
        }
    }

    fn record_initializer(
        &mut self,
        identifier: &Identifier,
        value: &Expression,
        origin: StatementOrigin,
        slot: usize,
    ) {
        if let Some(function) = reaching::function_expression(value) {
            let parameters: Vec<&TypedIdentifier> = function.iter_parameters().collect();
            self.record_literal(
                FunctionKey {
                    origin,
                    slot: slot as u32,
                },
                identifier,
                function.get_block(),
                &parameters,
            );
            return;
        }
        let Some(target) = identifier_expression(value) else {
            return;
        };
        let (Some(symbol), Some(target)) = (
            self.model.resolve_identifier(identifier),
            self.model.resolve_identifier(target),
        ) else {
            let unresolved = if self.model.resolve_identifier(identifier).is_none() {
                identifier
            } else {
                target
            };
            self.error = Some(InlineError::Reaching(ReachingError::UnresolvedIdentifier {
                name: unresolved.get_name().clone(),
                line: unresolved
                    .get_token()
                    .and_then(|token| token.get_line_number()),
            }));
            return;
        };
        if self.model.symbol(symbol).kind == SymbolKind::Local {
            self.aliases.entry(symbol).or_default().insert(target);
        }
    }

    fn record_literal(
        &mut self,
        key: FunctionKey,
        identifier: &Identifier,
        body: &Block,
        parameters: &[&TypedIdentifier],
    ) {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            self.error = Some(InlineError::Reaching(ReachingError::UnresolvedIdentifier {
                name: identifier.get_name().clone(),
                line: identifier
                    .get_token()
                    .and_then(|token| token.get_line_number()),
            }));
            return;
        };
        if self.model.symbol(symbol).kind != SymbolKind::Local {
            return;
        }
        self.literal_symbols.insert(symbol);
        let function = inlinable(self.model, symbol, body, parameters);
        self.functions.insert(key, function);
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

fn identifier_expression(expression: &Expression) -> Option<&Identifier> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier),
        Expression::Parenthese(parenthese) => identifier_expression(parenthese.inner_expression()),
        Expression::TypeCast(type_cast) => identifier_expression(type_cast.get_expression()),
        _ => None,
    }
}

/// Returns the captured local symbols that a nested function can write.
fn nested_writes(model: &SemanticModel) -> HashSet<SymbolId> {
    let mut result = HashSet::new();
    for symbol in &model.symbols {
        if symbol.kind != SymbolKind::Local || symbol.capturing_scopes.is_empty() {
            continue;
        }
        let declaring_function = model.function_scope_of(symbol.declaring_scope);
        if let Some(uses) = model.try_uses_of(symbol.id) {
            if uses.iter().any(|r#use| {
                r#use.kind == crate::semantic::AccessKind::Write
                    && model.function_scope_of(r#use.scope) != declaring_function
            }) {
                result.insert(symbol.id);
            }
        }
    }
    result
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
    functions: &'m HashMap<FunctionKey, Option<InlinableFunction>>,
    /// Local symbols that receive at least one function literal.
    literal_symbols: &'m HashSet<SymbolId>,
    /// Direct alias assignments.
    aliases: &'m HashMap<SymbolId, HashSet<SymbolId>>,
    /// The statement that contains each AST block.
    parents: &'m HashMap<BlockIdentity, (BlockIdentity, u32)>,
    /// The body block of each function region.
    function_roots: &'m HashMap<ScopeId, BlockIdentity>,
    /// The statement that contains each nested block.
    /// Function scopes whose bodies contain a `goto` or label.
    labelled: &'m HashSet<ScopeId>,
    /// The function scope that owns each block.
    block_functions: &'m HashMap<BlockIdentity, ScopeId>,
    /// Reaching definitions for each function region.
    regions: &'m HashMap<ScopeId, FunctionRegion>,
    /// Where each tracked symbol is defined.
    definition_locations: &'m HashMap<SymbolId, Vec<(ScopeId, reaching::DefinitionId)>>,
    /// Captured locals written by a nested function.
    nested_writes: &'m HashSet<SymbolId>,
    names: &'n mut NameGenerator,
    /// The scope of the block currently being rewritten.
    current_scope: Option<ScopeId>,
    /// The statement being rewritten, or the position after the block's last
    /// statement when rewriting its last statement.
    current_position: Option<StatementOrigin>,
    /// The branch condition whose expression is being rewritten, when the
    /// current call sits in an `if`, `while` or `repeat` condition.
    current_condition: Option<ConditionOrigin>,
    /// Whether the statement has not evaluated anything observable yet, so a
    /// call can be moved to the statement's start without reordering.
    clean: bool,
    /// Whether a call at the current expression position may be inlined. A
    /// repeat condition that cannot be rewritten falls back to this.
    allow_inline: bool,
    /// Whether escaping locals may be hoisted to the function start. Condition
    /// preambles disable this because they run inside the loop.
    hoist_locals: bool,
    /// Escaping locals that must be declared at the start of a function whose
    /// body contains `goto` or labels, keyed by the function scope.
    hoisted: HashMap<ScopeId, Vec<TypedIdentifier>>,
    inlined: &'i mut usize,
}

impl Inliner<'_, '_, '_> {
    fn rewrite_block(&mut self, block: &mut Block) {
        let previous_scope = self.current_scope;
        let previous_position = self.current_position;
        let previous_condition = self.current_condition;
        let previous_clean = self.clean;
        let previous_allow_inline = self.allow_inline;
        let previous_hoist = self.hoist_locals;
        let scope = self.model.try_scope_of(block).ok();
        self.current_scope = scope;
        self.current_condition = None;
        self.allow_inline = true;
        self.hoist_locals = true;
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

        if let Some(scope) = scope {
            if self.model.scope(scope).kind.is_function_boundary() {
                if let Some(declarations) = self.hoisted.remove(&scope) {
                    let mut statements = block.take_statements();
                    statements.insert(
                        0,
                        Statement::from(VariableAssignment::new(declarations, Vec::new())),
                    );
                    block.set_statements(statements);
                }
            }
        }

        self.current_scope = previous_scope;
        self.current_position = previous_position;
        self.current_condition = previous_condition;
        self.clean = previous_clean;
        self.allow_inline = previous_allow_inline;
        self.hoist_locals = previous_hoist;
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
                    let saved_condition = self.current_condition;
                    self.current_condition = self.current_position.map(|position| {
                        ConditionOrigin::new(position, ConditionKind::IfBranch(index as u32))
                    });
                    let mut condition = branch.get_condition().clone();
                    self.rewrite_single(&mut condition, preambles);
                    self.current_condition = saved_condition;
                    *branch.mutate_condition() = condition;
                    self.rewrite_block(branch.mutate_block());
                }
                if let Some(else_block) = statement.mutate_else_block() {
                    self.rewrite_block(else_block);
                }
            }
            Statement::While(statement) => {
                self.rewrite_while(statement, preambles);
            }
            Statement::Repeat(_) => {
                self.rewrite_repeat_statement(statement, preambles);
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

    /// Rewrites a loop or branch condition with reaching definitions taken from
    /// the condition's CFG entry.
    fn rewrite_condition(
        &mut self,
        condition: &Expression,
        kind: ConditionKind,
        scope: Option<ScopeId>,
        allow_inline: bool,
    ) -> (Expression, Vec<Statement>) {
        let saved_condition = self.current_condition;
        let saved_hoist = self.hoist_locals;
        let saved_scope = self.current_scope;
        let saved_clean = self.clean;
        let saved_allow_inline = self.allow_inline;

        self.current_condition = self
            .current_position
            .map(|position| ConditionOrigin::new(position, kind));
        self.hoist_locals = false;
        self.allow_inline = allow_inline;
        if let Some(scope) = scope {
            self.current_scope = Some(scope);
        }
        self.clean = true;

        let mut rewritten = condition.clone();
        let mut preambles = Vec::new();
        self.rewrite_single(&mut rewritten, &mut preambles);

        self.current_condition = saved_condition;
        self.hoist_locals = saved_hoist;
        self.current_scope = saved_scope;
        self.clean = saved_clean;
        self.allow_inline = saved_allow_inline;
        (rewritten, preambles)
    }

    /// Rewrites a `while` condition. The condition is evaluated before every
    /// iteration, so an inline preamble is placed at the top of a `while true`
    /// body followed by a break when the condition is false.
    fn rewrite_while(&mut self, statement: &mut WhileStatement, _preambles: &mut Vec<Statement>) {
        let (condition, condition_preambles) =
            self.rewrite_condition(statement.get_condition(), ConditionKind::While, None, true);

        self.rewrite_block(statement.mutate_block());

        if condition_preambles.is_empty() {
            *statement.mutate_condition() = condition;
            return;
        }

        let mut body = statement.get_block().clone();
        let mut statements = condition_preambles;
        statements.push(break_unless(condition));
        let original = body.take_statements();
        statements.extend(original);
        body.set_statements(statements);
        *statement = WhileStatement::new(body, Expression::from(true));
    }

    /// Rewrites a `repeat` condition. The condition is evaluated in the body's
    /// scope after every iteration. The inline preamble is appended at the end
    /// of the body and the condition becomes `until <result>`.
    ///
    /// A `continue` jumps to the condition, skipping the rest of the body but
    /// not the condition. Those statements are lowered structurally: each
    /// `continue` sets a flag, and the statements after it run only when the
    /// flag is clear. The condition section sits outside those guards, so it
    /// always runs. Body locals the condition reads stay in the body's scope;
    /// when a guard would otherwise move their declaration into a nested `if`,
    /// they are declared at the top of the body under fresh names and every
    /// reference bound to them is renamed.
    fn rewrite_repeat_statement(
        &mut self,
        statement: &mut Statement,
        _preambles: &mut Vec<Statement>,
    ) {
        let Statement::Repeat(repeat) = statement else {
            return;
        };
        let body_scope = self.model.try_scope_of(repeat.get_block()).ok();
        let original_condition = repeat.get_condition().clone();
        let before_condition = *self.inlined;
        let (mut condition, mut condition_preambles) =
            self.rewrite_condition(&original_condition, ConditionKind::Repeat, body_scope, true);
        let condition_inlined = *self.inlined - before_condition;

        let mut body = repeat.get_block().clone();
        self.rewrite_block(&mut body);

        if condition_preambles.is_empty() {
            *repeat.mutate_condition() = condition;
            repeat.mutate_block().clone_from(&body);
            return;
        }

        let Some(body_scope) = body_scope else {
            self.repeat_fallback(
                repeat,
                &mut body,
                &original_condition,
                condition_inlined,
                None,
            );
            return;
        };

        // The condition is unreachable when the body always returns or breaks
        // as its last statement. Appending after a trailing label would also
        // remove the label's block-end scope relaxation for a goto that relies
        // on it. Refuse those shapes rather than emit invalid or unreachable
        // code.
        if matches!(
            body.get_last_statement(),
            Some(LastStatement::Return(_)) | Some(LastStatement::Break(_))
        ) {
            self.repeat_fallback(
                repeat,
                &mut body,
                &original_condition,
                condition_inlined,
                Some(body_scope),
            );
            return;
        }

        let has_continue = block_contains_continue(&body);
        if has_continue
            && (body_has_close_local(&body)
                || body_contains_goto_or_label(&body)
                || body_declares_environment_local(&body, body_scope, self.model)
                || !self.hoist_repeat_condition_locals(
                    &mut body,
                    body_scope,
                    &mut condition,
                    &mut condition_preambles,
                ))
        {
            self.repeat_fallback(
                repeat,
                &mut body,
                &original_condition,
                condition_inlined,
                Some(body_scope),
            );
            return;
        }

        if has_continue {
            let flag = self.names.generate();
            let mut statements = body.take_statements();
            statements.insert(
                0,
                Statement::from(VariableAssignment::new(
                    vec![TypedIdentifier::new(flag.clone())],
                    vec![Expression::from(false)],
                )),
            );
            body.set_statements(statements);
            lower_repeat_continues(&mut body, &flag);
        }

        let result = self.names.generate();
        let mut statements = body.take_statements();
        statements.extend(condition_preambles);
        statements.push(Statement::from(VariableAssignment::new(
            vec![TypedIdentifier::new(result.clone())],
            vec![condition],
        )));
        body.set_statements(statements);
        *repeat.mutate_condition() = Expression::identifier(result);
        repeat.mutate_block().clone_from(&body);
    }

    /// Restores a repeat condition that could not be rewritten, rewriting the
    /// original expression without inlining. The body is already rewritten.
    fn repeat_fallback(
        &mut self,
        repeat: &mut RepeatStatement,
        body: &mut Block,
        original_condition: &Expression,
        condition_inlined: usize,
        body_scope: Option<ScopeId>,
    ) {
        *self.inlined -= condition_inlined;
        let (fallback, fallback_preambles) =
            self.rewrite_condition(original_condition, ConditionKind::Repeat, body_scope, false);
        debug_assert!(
            fallback_preambles.is_empty(),
            "a non-inlining condition rewrite must not produce a preamble"
        );
        *repeat.mutate_condition() = fallback;
        repeat.mutate_block().clone_from(body);
    }

    /// Moves the body locals read by a repeat condition to the top of the loop
    /// body under fresh names and renames every bound reference. Returns false
    /// when a declaration cannot be moved safely.
    fn hoist_repeat_condition_locals(
        &mut self,
        body: &mut Block,
        body_scope: ScopeId,
        condition: &mut Expression,
        condition_preambles: &mut Vec<Statement>,
    ) -> bool {
        let mut reads = BodyScopedReads {
            model: self.model,
            scope: body_scope,
            symbols: HashSet::new(),
        };
        DefaultVisitor::visit_expression(condition, &mut reads);
        let mut preamble_block = Block::new(std::mem::take(condition_preambles), None);
        DefaultVisitor::visit_block(&mut preamble_block, &mut reads);
        *condition_preambles = preamble_block.take_statements();

        let visible = reads.symbols;
        if visible.is_empty() {
            return true;
        }

        // Find the declaration of every visible symbol. A visible symbol is a
        // top-level local of the repeat body, so its declaration is either a
        // `local` assignment or a `local function`.
        let statements = body.take_statements();
        let mut hoisted_symbols: HashSet<SymbolId> = HashSet::new();
        let mut renames: HashMap<SymbolId, String> = HashMap::new();
        let mut safe = true;

        for statement in &statements {
            match statement {
                Statement::LocalAssign(assignment) => {
                    let identifiers: Vec<&Identifier> = assignment
                        .iter_variables()
                        .map(|v| v.get_identifier())
                        .collect();
                    let symbols: Vec<Option<SymbolId>> = identifiers
                        .iter()
                        .map(|identifier| self.model.resolve_identifier(identifier))
                        .collect();
                    let group_visible = symbols
                        .iter()
                        .any(|symbol| symbol.is_some_and(|symbol| visible.contains(&symbol)));
                    if !group_visible {
                        continue;
                    }
                    for (variable, symbol) in assignment.iter_variables().zip(&symbols) {
                        let Some(symbol) = *symbol else {
                            safe = false;
                            break;
                        };
                        if variable.get_attribute() == Some("close") {
                            safe = false;
                            break;
                        }
                        hoisted_symbols.insert(symbol);
                        renames
                            .entry(symbol)
                            .or_insert_with(|| self.names.generate());
                    }
                    if !safe {
                        break;
                    }
                }
                Statement::LocalFunction(function) => {
                    let Some(symbol) = self.model.resolve_identifier(function.get_identifier())
                    else {
                        safe = false;
                        break;
                    };
                    if !visible.contains(&symbol) {
                        continue;
                    }
                    hoisted_symbols.insert(symbol);
                    renames
                        .entry(symbol)
                        .or_insert_with(|| self.names.generate());
                }
                _ => {}
            }
        }

        if !safe {
            body.set_statements(statements);
            return false;
        }

        if visible.iter().any(|symbol| !renames.contains_key(symbol)) {
            body.set_statements(statements);
            return false;
        }

        let mut transformed = Vec::with_capacity(statements.len());
        for statement in statements {
            match statement {
                Statement::LocalAssign(assignment) => {
                    let hoisted = assignment.iter_variables().any(|variable| {
                        self.model
                            .resolve_identifier(variable.get_identifier())
                            .is_some_and(|symbol| hoisted_symbols.contains(&symbol))
                    });
                    if !hoisted {
                        transformed.push(Statement::LocalAssign(assignment));
                        continue;
                    }
                    let values: Vec<Expression> = assignment.iter_values().cloned().collect();
                    if values.is_empty() {
                        continue;
                    }
                    let variables: Vec<Variable> = assignment
                        .iter_variables()
                        .map(|variable| {
                            let symbol = self
                                .model
                                .resolve_identifier(variable.get_identifier())
                                .expect("hoisted declarations resolve");
                            Variable::Identifier(Identifier::new(
                                renames
                                    .get(&symbol)
                                    .expect("every hoisted variable has a name")
                                    .clone(),
                            ))
                        })
                        .collect();
                    transformed.push(Statement::from(AssignStatement::new(variables, values)));
                }
                Statement::LocalFunction(function) => {
                    let symbol = self
                        .model
                        .resolve_identifier(function.get_identifier())
                        .expect("the declaration scan resolved every local function");
                    if !hoisted_symbols.contains(&symbol) {
                        transformed.push(Statement::LocalFunction(function));
                        continue;
                    }
                    let name = renames
                        .get(&symbol)
                        .expect("every hoisted function has a name")
                        .clone();
                    let mut expression = crate::nodes::FunctionExpression::new(
                        function.get_block().clone(),
                        function.iter_parameters().cloned().collect(),
                        function.is_variadic(),
                    );
                    *expression.mutate_attributes() = function.attributes().clone();
                    if let Some(generic) = function.get_generic_parameters() {
                        expression.set_generic_parameters(generic.clone());
                    }
                    if let Some(variadic) = function.get_variadic_type() {
                        expression.set_variadic_type(variadic.clone());
                    }
                    if let Some(return_type) = function.get_return_type() {
                        expression.set_return_type(return_type.clone());
                    }
                    transformed.push(Statement::from(AssignStatement::new(
                        vec![Variable::Identifier(Identifier::new(name))],
                        vec![Expression::Function(Box::new(expression))],
                    )));
                }
                other => transformed.push(other),
            }
        }

        body.set_statements(transformed);

        let mut names: Vec<&String> = renames.values().collect();
        names.sort();
        let declarations: Vec<TypedIdentifier> = names
            .into_iter()
            .map(|name| TypedIdentifier::new(name.clone()))
            .collect();
        let mut statements = body.take_statements();
        statements.insert(
            0,
            Statement::from(VariableAssignment::new(declarations, Vec::new())),
        );
        body.set_statements(statements);

        let mut renamer = BindingRenamer {
            model: self.model,
            renames: &renames,
        };
        DefaultVisitor::visit_block(body, &mut renamer);
        DefaultVisitor::visit_expression(condition, &mut renamer);
        let mut preamble_block = Block::new(std::mem::take(condition_preambles), None);
        DefaultVisitor::visit_block(&mut preamble_block, &mut renamer);
        *condition_preambles = preamble_block.take_statements();

        true
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
        let mut later_may_inline = vec![false; length];
        let mut seen = false;
        for index in (0..length).rev() {
            later_may_inline[index] = seen;
            seen = seen || self.expression_may_inline(&original[index]);
        }
        let mut rewritten = Vec::with_capacity(length);
        for (index, value) in original.iter().enumerate() {
            let later_may_inline = later_may_inline[index];
            let clean = self.clean;
            let tail = multi_tail && index + 1 == length;
            let results = self.rewrite(value.clone(), tail, preambles);
            if later_may_inline && clean && self.allow_inline && self.position_allows_insertion() {
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
    ///
    /// A method call needs one more step. PUC Lua evaluates the receiver and
    /// performs the method lookup before the arguments; Luau evaluates the
    /// receiver, then the arguments, and only then performs the lookup. When an
    /// argument may inline, the receiver is stored before the body, and for PUC
    /// Lua the lookup is stored there too. The call is then a normal call to the
    /// stored method with the stored receiver as its first argument. Luau keeps
    /// the method syntax with the stored receiver, so the lookup stays at the
    /// call. An undetermined dialect only permits this when the receiver is a
    /// fresh table literal, whose lookup cannot run code.
    fn rewrite_call_contents(
        &mut self,
        call: &mut crate::nodes::FunctionCall,
        preambles: &mut Vec<Statement>,
    ) {
        let arguments: Vec<Expression> = call.get_arguments().clone().to_expressions();
        let method = call.get_method().cloned();
        let argument_may_inline = arguments
            .iter()
            .any(|argument| self.expression_may_inline(argument));

        if let Some(method) = method {
            if argument_may_inline
                && self.clean
                && self.allow_inline
                && self.position_allows_insertion()
            {
                if let Some(lookup_before_arguments) = self.method_lookup_order() {
                    let mut prefix = call.get_prefix().clone();
                    self.rewrite_prefix(&mut prefix, preambles);
                    let receiver = self.materialize_prefix(prefix, preambles);
                    self.clean = true;
                    if lookup_before_arguments {
                        let field = crate::nodes::FieldExpression::new(
                            receiver.clone(),
                            Identifier::new(method.get_name().clone()),
                        );
                        let method_value =
                            self.materialize(Expression::Field(Box::new(field)), preambles);
                        let mut values = arguments;
                        self.rewrite_values(&mut values, true, preambles);
                        let mut call_values = Vec::with_capacity(values.len() + 1);
                        call_values.push(prefix_into_expression(receiver));
                        call_values.extend(values);
                        *call = crate::nodes::FunctionCall::new(
                            prefix_from_expression(method_value),
                            crate::nodes::Arguments::Tuple(
                                crate::nodes::TupleArguments::from_iter(call_values),
                            ),
                            None,
                        );
                    } else {
                        let mut values = arguments;
                        self.rewrite_values(&mut values, true, preambles);
                        *call.mutate_prefix() = receiver;
                        *call.mutate_arguments() = crate::nodes::Arguments::Tuple(
                            crate::nodes::TupleArguments::from_iter(values),
                        );
                    }
                    return;
                }

                if !receiver_lookup_is_inert(call.get_prefix()) {
                    // `Dialect::Default` has no fixed method lookup order.
                    return;
                }
                let mut prefix = call.get_prefix().clone();
                self.rewrite_prefix(&mut prefix, preambles);
                let receiver = self.materialize_prefix(prefix, preambles);
                self.clean = true;
                let mut values = arguments;
                self.rewrite_values(&mut values, true, preambles);
                *call.mutate_prefix() = receiver;
                *call.mutate_arguments() =
                    crate::nodes::Arguments::Tuple(crate::nodes::TupleArguments::from_iter(values));
                return;
            }

            let mut prefix = call.get_prefix().clone();
            self.rewrite_prefix(&mut prefix, preambles);
            *call.mutate_prefix() = prefix;
            let mut values = arguments;
            self.rewrite_values(&mut values, true, preambles);
            *call.mutate_arguments() =
                crate::nodes::Arguments::Tuple(crate::nodes::TupleArguments::from_iter(values));
            return;
        }

        let spill = arguments
            .iter()
            .any(|argument| self.expression_may_inline(argument));
        let prefix_clean = self.clean;
        let mut prefix = call.get_prefix().clone();
        self.rewrite_prefix(&mut prefix, preambles);
        if spill && self.allow_inline && prefix_clean && self.position_allows_insertion() {
            prefix = self.materialize_prefix(prefix, preambles);
            self.clean = true;
        }
        *call.mutate_prefix() = prefix;
        let mut values = arguments;
        self.rewrite_values(&mut values, true, preambles);
        *call.mutate_arguments() =
            crate::nodes::Arguments::Tuple(crate::nodes::TupleArguments::from_iter(values));
    }

    /// Returns whether the method lookup precedes the argument evaluation.
    ///
    /// PUC Lua and LuaJIT perform `SELF` (receiver and lookup) before the
    /// arguments. Luau emits `NAMECALL` after the arguments, so its lookup runs
    /// last. `Dialect::Default` is undetermined and returns `None`.
    fn method_lookup_order(&self) -> Option<bool> {
        match self.model.dialect() {
            crate::Dialect::Lua51
            | crate::Dialect::Lua52
            | crate::Dialect::Lua53
            | crate::Dialect::Lua54
            | crate::Dialect::LuaJit => Some(true),
            crate::Dialect::Luau => Some(false),
            crate::Dialect::Default => None,
        }
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
        if let Some(scope) = self.hoist_target() {
            // A label in the same function could be jumped over; keep the
            // declaration at the function start so the jump never enters its
            // scope. The value is still computed here.
            self.hoisted
                .entry(scope)
                .or_default()
                .push(TypedIdentifier::new(name.clone()));
            preambles.push(Statement::from(AssignStatement::new(
                vec![Variable::Identifier(Identifier::new(name.clone()))],
                vec![expression],
            )));
        } else {
            preambles.push(Statement::from(VariableAssignment::new(
                vec![TypedIdentifier::new(name.clone())],
                vec![expression],
            )));
        }
        Expression::identifier(name)
    }

    /// Returns the function scope whose start must receive escaping locals,
    /// when the current function contains a `goto` or label.
    fn hoist_target(&self) -> Option<ScopeId> {
        if !self.hoist_locals {
            return None;
        }
        let position = self.current_position?;
        let function = *self.block_functions.get(&position.block)?;
        self.labelled.contains(&function).then_some(function)
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

    /// Returns whether a new local can be declared at the current position.
    ///
    /// Locals that escape the preamble are hoisted to the start of functions
    /// containing labels, so inserting at the statement itself is always safe
    /// for the statements this pass rewrites.
    fn position_allows_insertion(&self) -> bool {
        let Some(position) = self.current_position else {
            return false;
        };
        self.block_functions.contains_key(&position.block)
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
                if !conditional
                    && right_may_inline
                    && clean
                    && self.allow_inline
                    && self.position_allows_insertion()
                {
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
                if key_may_inline && clean && self.allow_inline && self.position_allows_insertion()
                {
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
                let mut later_may_inline = vec![false; length];
                let mut seen = false;
                for index in (0..length).rev() {
                    later_may_inline[index] = seen;
                    seen = seen || self.table_entry_may_inline(&entries[index]);
                }
                let mut replacement: Vec<crate::nodes::TableEntry> = Vec::with_capacity(length);
                for (index, entry) in entries.iter().enumerate() {
                    let later_may_inline = spillable && later_may_inline[index];
                    let clean = self.clean;
                    let tail = index + 1 == length;
                    match entry.clone() {
                        crate::nodes::TableEntry::Field(mut field) => {
                            let mut value = field.get_value().clone();
                            self.rewrite_single(&mut value, preambles);
                            if later_may_inline
                                && clean
                                && self.allow_inline
                                && self.position_allows_insertion()
                            {
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
                                && self.allow_inline
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
                                && self.allow_inline
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
                                    && self.allow_inline
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
        if !self.clean || !self.allow_inline || call.get_method().is_some() {
            return None;
        }
        if !self.position_allows_insertion() {
            return None;
        }
        let callee = self.model.resolve_identifier(callee(call.get_prefix())?)?;
        let key = self.resolve_call_function(callee)?;
        let function = self.functions.get(&key)?.as_ref()?.clone();
        if tail && !function.uniform {
            return None;
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
        if !rename_labels(&mut body, self.model, self.names) {
            return None;
        }
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
            if let Some(scope) = self.hoist_target() {
                self.hoisted.entry(scope).or_default().extend(declarations);
            } else {
                preambles.push(Statement::from(VariableAssignment::new(
                    declarations,
                    Vec::new(),
                )));
            }
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

    /// Resolves the function key that `symbol` holds at the current position.
    ///
    /// Direct calls use the reaching definitions of the enclosing function.
    /// A call inside a nested function reads an upvalue; only an immutable
    /// symbol has one fixed value, so a mutable upvalue is refused. The alias
    /// chain is followed at the position each alias captured its value, not at
    /// the call site.
    fn resolve_call_function(&self, symbol: SymbolId) -> Option<FunctionKey> {
        // A captured local written by a nested function can change whenever
        // that function runs, and the enclosing CFG does not model those calls.
        if self.model.symbol(symbol).write_count > 1 && self.nested_writes.contains(&symbol) {
            return None;
        }
        let position = self.current_position?;
        let region_scope = *self.block_functions.get(&position.block)?;
        let declaring_scope = self.model.symbol(symbol).declaring_scope;
        let declaring_function = self.model.function_scope_of(declaring_scope)?;

        if declaring_function == region_scope {
            let definitions = self.definitions_at(region_scope, position)?;
            self.resolve_definitions(symbol, definitions, region_scope, &mut HashSet::new())
        } else if self.model.is_immutable(symbol) {
            let locations = self.definition_locations.get(&symbol)?;
            if locations.len() != 1 {
                return None;
            }
            let (scope, id) = locations[0];
            self.resolve_definition(scope, id, symbol, &mut HashSet::new())
        } else {
            self.resolve_captured_function(symbol, region_scope)
        }
    }

    /// Resolves a mutable upvalue read by a nested function.
    ///
    /// The upvalue cell is captured when the nested function is created. If no
    /// write to it can run after that creation, every later call observes the
    /// value that reached the creation statement. Only a function created
    /// directly inside the declaring function is considered; an intermediate
    /// function could be called before or after later writes, so that shape is
    /// refused.
    fn resolve_captured_function(
        &self,
        symbol: SymbolId,
        call_region: ScopeId,
    ) -> Option<FunctionKey> {
        let declaring_scope = self.model.symbol(symbol).declaring_scope;
        let declaring_function = self.model.function_scope_of(declaring_scope)?;

        // Walk out through every enclosing function until reaching the
        // function that declares the symbol. The creation of the outermost
        // closure in that chain is the first point at which later writes in
        // the declaring function can affect the value the call sees.
        let mut region = call_region;
        let mut creation = None;
        for _ in 0..self.model.scopes.len() {
            let root = *self.function_roots.get(&region)?;
            let (block, index) = *self.parents.get(&root)?;
            let candidate = StatementOrigin { block, index };
            let parent = *self.block_functions.get(&candidate.block)?;
            if parent == declaring_function {
                creation = Some(candidate);
                break;
            }
            if parent == region {
                return None;
            }
            region = parent;
        }
        let creation = creation?;

        let region = self.regions.get(&declaring_function)?;
        if region.has_write_after(symbol, creation)? {
            return None;
        }
        let definitions = region.at_statement(creation)?;
        self.resolve_definitions(symbol, definitions, declaring_function, &mut HashSet::new())
    }

    /// Returns the reaching definitions at the current position within
    /// `region_scope`.
    fn definitions_at(
        &self,
        region_scope: ScopeId,
        position: StatementOrigin,
    ) -> Option<&[reaching::DefinitionId]> {
        let region = self.regions.get(&region_scope)?;
        if let Some(condition) = self.current_condition {
            if let Some(definitions) = region.at_condition(condition) {
                return Some(definitions);
            }
        }
        if let Some(definitions) = region.at_statement(position) {
            return Some(definitions);
        }
        if let Some(definitions) = region.at_loop_initial(position) {
            return Some(definitions);
        }
        region.at_terminator(position)
    }

    fn resolve_definition(
        &self,
        region_scope: ScopeId,
        id: reaching::DefinitionId,
        symbol: SymbolId,
        visited: &mut HashSet<(SymbolId, StatementOrigin)>,
    ) -> Option<FunctionKey> {
        self.resolve_definitions(symbol, &[id], region_scope, visited)
    }

    /// Resolves a reaching set to one function that every reaching definition
    /// agrees on.
    fn resolve_definitions(
        &self,
        symbol: SymbolId,
        definitions: &[reaching::DefinitionId],
        region_scope: ScopeId,
        visited: &mut HashSet<(SymbolId, StatementOrigin)>,
    ) -> Option<FunctionKey> {
        let region = self.regions.get(&region_scope)?;
        let relevant = region.definitions_of(definitions, symbol);
        if relevant.is_empty() {
            return None;
        }

        let mut chosen: Option<(FunctionKey, InlinableFunction)> = None;
        for id in relevant {
            let definition = region.definition(id);
            let key = match &definition.value {
                DefinitionValue::Function(key) => *key,
                DefinitionValue::Alias(target) => {
                    self.resolve_alias(*target, definition.origin, region_scope, visited)?
                }
                DefinitionValue::Other => return None,
            };
            let function = self.functions.get(&key)?.as_ref()?.clone();
            match &chosen {
                Some((_, previous)) if !previous.same_function(&function) => return None,
                Some(_) => {}
                None => chosen = Some((key, function)),
            }
        }
        chosen.map(|(key, _)| key)
    }

    /// Resolves the value captured by `target` at the alias statement.
    fn resolve_alias(
        &self,
        target: SymbolId,
        origin: StatementOrigin,
        region_scope: ScopeId,
        visited: &mut HashSet<(SymbolId, StatementOrigin)>,
    ) -> Option<FunctionKey> {
        if !visited.insert((target, origin)) {
            return None;
        }
        let target_scope = self.model.symbol(target).declaring_scope;
        let target_function = self.model.function_scope_of(target_scope)?;
        if target_function == region_scope {
            let region = self.regions.get(&region_scope)?;
            let definitions = region.at_statement(origin)?;
            self.resolve_definitions(target, definitions, region_scope, visited)
        } else if self.model.is_immutable(target) {
            let locations = self.definition_locations.get(&target)?;
            if locations.len() != 1 {
                return None;
            }
            let (scope, id) = locations[0];
            self.resolve_definition(scope, id, target, visited)
        } else {
            self.resolve_captured_function(target, region_scope)
        }
    }

    /// Returns whether `symbol` can ever denote one of the recorded functions.
    fn may_resolve_function(&self, symbol: SymbolId) -> bool {
        let mut visited = HashSet::new();
        self.may_resolve_function_inner(symbol, &mut visited)
    }

    fn may_resolve_function_inner(
        &self,
        symbol: SymbolId,
        visited: &mut HashSet<SymbolId>,
    ) -> bool {
        if !visited.insert(symbol) {
            return false;
        }
        if self.literal_symbols.contains(&symbol) {
            return true;
        }
        self.aliases.get(&symbol).is_some_and(|targets| {
            targets
                .iter()
                .any(|target| self.may_resolve_function_inner(*target, visited))
        })
    }

    /// Returns whether reading `symbol` as a callee is stable: it resolves to a
    /// local function value that a later inline body in the same expression
    /// cannot change.
    fn callee_binding_is_stable(&self, symbol: SymbolId) -> bool {
        if !self.may_resolve_function(symbol) {
            return false;
        }
        let info = self.model.symbol(symbol);
        info.write_count <= 1
            || info.capturing_scopes.is_empty()
            || !self.nested_writes.contains(&symbol)
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
        self.may_resolve_function(symbol)
    }

    /// Returns whether reading `identifier` as a callee is stable: it resolves
    /// to a local function value that a later inline body cannot replace.
    fn callee_is_inlinable(&self, identifier: &Identifier) -> bool {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return false;
        };
        self.callee_binding_is_stable(symbol)
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

/// Returns whether a block contains a `continue` that targets this loop.
///
/// Continues inside a nested loop or a nested function target another loop and
/// do not count.
fn block_contains_continue(block: &Block) -> bool {
    block.iter_statements().any(statement_contains_continue)
        || matches!(block.get_last_statement(), Some(LastStatement::Continue(_)))
}

fn statement_contains_continue(statement: &Statement) -> bool {
    match statement {
        Statement::Do(statement) => block_contains_continue(statement.get_block()),
        Statement::If(statement) => {
            statement
                .iter_branches()
                .any(|branch| block_contains_continue(branch.get_block()))
                || statement
                    .get_else_block()
                    .is_some_and(block_contains_continue)
        }
        _ => false,
    }
}

/// Returns whether a body contains a `goto` or label outside nested functions.
///
/// A `continue` only exists in Luau, which has no labels, but an undetermined
/// dialect can parse both. Guarding statements after a `continue` would move
/// labels into nested blocks, so that combination is refused.
fn body_contains_goto_or_label(block: &Block) -> bool {
    block
        .iter_statements()
        .any(statement_contains_goto_or_label)
}

fn statement_contains_goto_or_label(statement: &Statement) -> bool {
    match statement {
        Statement::Goto(_) | Statement::Label(_) => true,
        Statement::Do(statement) => body_contains_goto_or_label(statement.get_block()),
        Statement::If(statement) => {
            statement
                .iter_branches()
                .any(|branch| body_contains_goto_or_label(branch.get_block()))
                || statement
                    .get_else_block()
                    .is_some_and(body_contains_goto_or_label)
        }
        Statement::While(statement) => body_contains_goto_or_label(statement.get_block()),
        Statement::Repeat(statement) => body_contains_goto_or_label(statement.get_block()),
        Statement::NumericFor(statement) => body_contains_goto_or_label(statement.get_block()),
        Statement::GenericFor(statement) => body_contains_goto_or_label(statement.get_block()),
        _ => false,
    }
}

/// Returns whether a repeat body declares a top-level local named `_ENV`.
///
/// Guarding statements after a `continue` can shrink that local's scope, which
/// would change the environment the appended condition uses. Only a directly
/// declared body-scope `_ENV` is affected; nested blocks already ended before
/// the condition.
fn body_declares_environment_local(
    block: &Block,
    body_scope: ScopeId,
    model: &SemanticModel,
) -> bool {
    block.iter_statements().any(|statement| match statement {
        Statement::LocalAssign(assignment) => assignment.iter_variables().any(|variable| {
            model
                .resolve_identifier(variable.get_identifier())
                .is_some_and(|symbol| {
                    model.symbol(symbol).declaring_scope == body_scope
                        && model.symbol(symbol).name == "_ENV"
                })
        }),
        Statement::LocalFunction(function) => model
            .resolve_identifier(function.get_identifier())
            .is_some_and(|symbol| {
                model.symbol(symbol).declaring_scope == body_scope
                    && model.symbol(symbol).name == "_ENV"
            }),
        _ => false,
    })
}

/// Returns whether a block owns a `<close>` local whose closing point the
/// structural `continue` lowering could move. Nested loops and functions keep
/// their own scope and are not inspected.
fn body_has_close_local(block: &Block) -> bool {
    let mut found = false;
    for statement in block.iter_statements() {
        match statement {
            Statement::LocalAssign(assignment) => {
                if assignment
                    .iter_variables()
                    .any(|variable| variable.get_attribute() == Some("close"))
                {
                    found = true;
                }
            }
            Statement::Do(statement) => found |= body_has_close_local(statement.get_block()),
            Statement::If(statement) => {
                found |= statement
                    .iter_branches()
                    .any(|branch| body_has_close_local(branch.get_block()));
                if let Some(else_block) = statement.get_else_block() {
                    found |= body_has_close_local(else_block);
                }
            }
            _ => {}
        }
    }
    found
}

/// Replaces every `continue` targeting this loop with a flag assignment and
/// guards the statements that follow it with `if not flag then ... end`.
fn lower_repeat_continues(block: &mut Block, flag: &str) {
    let mut statements = block.take_statements();
    if let Some(last) = block.take_last_statement() {
        if matches!(last, LastStatement::Continue(_)) {
            statements.push(continue_assignment(flag));
        } else {
            block.set_last_statement(last);
        }
    }
    for statement in &mut statements {
        lower_repeat_continue_statement(statement, flag);
    }
    block.set_statements(guard_statements(statements, flag));
}

fn lower_repeat_continue_statement(statement: &mut Statement, flag: &str) {
    match statement {
        Statement::Do(statement) => lower_repeat_continues(statement.mutate_block(), flag),
        Statement::If(statement) => {
            for branch in statement.mutate_branches() {
                lower_repeat_continues(branch.mutate_block(), flag);
            }
            if let Some(else_block) = statement.mutate_else_block() {
                lower_repeat_continues(else_block, flag);
            }
        }
        _ => {}
    }
}

fn continue_assignment(flag: &str) -> Statement {
    Statement::from(AssignStatement::new(
        vec![Variable::Identifier(Identifier::new(flag.to_owned()))],
        vec![Expression::from(true)],
    ))
}

/// Builds the guarded statement sequence for one block. After the first
/// statement that may continue, all following statements run only while the
/// flag is clear.
fn guard_statements(statements: Vec<Statement>, flag: &str) -> Vec<Statement> {
    let mut result = Vec::with_capacity(statements.len());
    let mut iterator = statements.into_iter();
    while let Some(statement) = iterator.next() {
        let may_continue = statement_may_continue(&statement, flag);
        result.push(statement);
        if may_continue {
            let rest: Vec<Statement> = iterator.collect();
            if !rest.is_empty() {
                let guarded = guard_statements(rest, flag);
                result.push(guard_block(guarded, flag));
            }
            break;
        }
    }
    result
}

fn guard_block(statements: Vec<Statement>, flag: &str) -> Statement {
    Statement::from(crate::nodes::IfStatement::new(
        vec![crate::nodes::IfBranch::new(
            Expression::from(UnaryExpression::new(
                UnaryOperator::Not,
                Expression::identifier(flag.to_owned()),
            )),
            Block::new(statements, None),
        )],
        None,
    ))
}

fn statement_may_continue(statement: &Statement, flag: &str) -> bool {
    match statement {
        Statement::Assign(assignment) => assignment.iter_variables().any(|variable| {
            matches!(variable, Variable::Identifier(identifier) if identifier.get_name() == flag)
        }),
        Statement::Do(statement) => block_may_continue(statement.get_block(), flag),
        Statement::If(statement) => {
            statement
                .iter_branches()
                .any(|branch| block_may_continue(branch.get_block(), flag))
                || statement
                    .get_else_block()
                    .is_some_and(|block| block_may_continue(block, flag))
        }
        _ => false,
    }
}

fn block_may_continue(block: &Block, flag: &str) -> bool {
    block
        .iter_statements()
        .any(|statement| statement_may_continue(statement, flag))
        || matches!(block.get_last_statement(), Some(LastStatement::Continue(_)))
}

/// Builds `if not condition then break end`.
fn break_unless(condition: Expression) -> Statement {
    Statement::from(crate::nodes::IfStatement::new(
        vec![crate::nodes::IfBranch::new(
            Expression::from(UnaryExpression::new(UnaryOperator::Not, condition)),
            Block::new(vec![], Some(LastStatement::Break(None))),
        )],
        None,
    ))
}

/// Collects the symbols declared directly in one block scope that an
/// expression reads. These are the body locals a repeat condition can see.
struct BodyScopedReads<'m> {
    model: &'m SemanticModel,
    scope: ScopeId,
    symbols: HashSet<SymbolId>,
}

impl NodeProcessor for BodyScopedReads<'_> {
    fn process_identifier(&mut self, identifier: &mut Identifier) {
        if let Some(symbol) = self.model.resolve_identifier(identifier) {
            if self.model.symbol(symbol).declaring_scope == self.scope {
                self.symbols.insert(symbol);
            }
        }
    }
}

/// Renames every identifier occurrence bound to one of `renames`.
struct BindingRenamer<'m, 'r> {
    model: &'m SemanticModel,
    renames: &'r HashMap<SymbolId, String>,
}

impl NodeProcessor for BindingRenamer<'_, '_> {
    fn process_identifier(&mut self, identifier: &mut Identifier) {
        if let Some(symbol) = self.model.resolve_identifier(identifier) {
            if let Some(name) = self.renames.get(&symbol) {
                identifier.set_name(name.clone());
            }
        }
    }
}

/// Gives every label in a function body a fresh name, with its `goto`s.
///
/// Labels and goto sites are keyed by their structural statement positions from
/// the semantic model, so a cloned body cannot pair a goto with the wrong
/// target. Names are checked through the model and a position that is missing
/// or mismatched refuses the inline instead of guessing.
fn rename_labels(block: &mut Block, model: &SemanticModel, names: &mut NameGenerator) -> bool {
    let Ok(scope) = model.try_scope_of(block) else {
        return false;
    };
    let Some(function) = model.function_scope_of(scope) else {
        return false;
    };

    let mut renames: HashMap<LabelId, String> = HashMap::new();
    let mut labels: HashMap<StatementPosition, LabelId> = HashMap::new();
    let mut targets: HashMap<StatementPosition, (String, LabelId)> = HashMap::new();
    for label in model.labels_in_function(function) {
        renames.insert(label.id, names.generate());
        labels.insert(label.position, label.id);
    }
    for binding in model.goto_bindings_in_function(function) {
        targets.insert(
            binding.site.position,
            (binding.site.name.clone(), binding.label),
        );
    }

    struct Renamer<'a, 'm> {
        model: &'m SemanticModel,
        renames: &'a HashMap<LabelId, String>,
        labels: &'a HashMap<StatementPosition, LabelId>,
        targets: &'a HashMap<StatementPosition, (String, LabelId)>,
        label_count: usize,
        goto_count: usize,
    }

    impl Renamer<'_, '_> {
        fn apply(&mut self, block: &mut Block) -> bool {
            let Some(identity) = block.get_identity() else {
                return false;
            };
            for (index, statement) in block.iter_mut_statements().enumerate() {
                let position = StatementPosition {
                    block: identity,
                    index: index as u32,
                };
                match statement {
                    Statement::Label(label) => {
                        let Some(&id) = self.labels.get(&position) else {
                            return false;
                        };
                        if self.model.label(id).name.as_str() != label.get_label_name() {
                            return false;
                        }
                        let Some(name) = self.renames.get(&id) else {
                            return false;
                        };
                        label.mutate_label().set_name(name.clone());
                        self.label_count += 1;
                    }
                    Statement::Goto(goto) => {
                        let Some((name, target)) = self.targets.get(&position) else {
                            return false;
                        };
                        if name.as_str() != goto.get_label_name() {
                            return false;
                        }
                        let Some(new_name) = self.renames.get(target) else {
                            return false;
                        };
                        goto.mutate_label().set_name(new_name.clone());
                        self.goto_count += 1;
                    }
                    Statement::Do(statement) => {
                        if !self.apply(statement.mutate_block()) {
                            return false;
                        }
                    }
                    Statement::While(statement) => {
                        if !self.apply(statement.mutate_block()) {
                            return false;
                        }
                    }
                    Statement::Repeat(statement) => {
                        if !self.apply(statement.mutate_block()) {
                            return false;
                        }
                    }
                    Statement::NumericFor(statement) => {
                        if !self.apply(statement.mutate_block()) {
                            return false;
                        }
                    }
                    Statement::GenericFor(statement) => {
                        if !self.apply(statement.mutate_block()) {
                            return false;
                        }
                    }
                    Statement::If(statement) => {
                        for branch in statement.mutate_branches() {
                            if !self.apply(branch.mutate_block()) {
                                return false;
                            }
                        }
                        if let Some(else_block) = statement.mutate_else_block() {
                            if !self.apply(else_block) {
                                return false;
                            }
                        }
                    }
                    _ => {}
                }
            }
            true
        }
    }

    let mut renamer = Renamer {
        model,
        renames: &renames,
        labels: &labels,
        targets: &targets,
        label_count: 0,
        goto_count: 0,
    };
    renamer.apply(block)
        && renamer.label_count == labels.len()
        && renamer.goto_count == targets.len()
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

fn prefix_from_expression(expression: Expression) -> Prefix {
    match expression {
        Expression::Call(call) => Prefix::Call(call),
        Expression::Field(field) => Prefix::Field(field),
        Expression::Identifier(identifier) => Prefix::Identifier(identifier),
        Expression::Index(index) => Prefix::Index(index),
        Expression::Parenthese(parenthese) => Prefix::Parenthese(parenthese),
        Expression::TypeInstantiation(instantiation) => Prefix::TypeInstantiation(instantiation),
        expression => Prefix::Parenthese(Box::new(crate::nodes::ParentheseExpression::new(
            expression,
        ))),
    }
}

/// Returns whether looking up a method on `prefix` cannot run user code or
/// raise. Only a fresh table literal has no metatable and no string/library
/// fallback.
fn receiver_lookup_is_inert(prefix: &Prefix) -> bool {
    match prefix {
        Prefix::Parenthese(parenthese) => {
            receiver_lookup_expression_is_inert(parenthese.inner_expression())
        }
        Prefix::TypeInstantiation(instantiation) => {
            receiver_lookup_is_inert(instantiation.get_prefix())
        }
        _ => false,
    }
}

fn receiver_lookup_expression_is_inert(expression: &Expression) -> bool {
    match expression {
        Expression::Table(_) => true,
        Expression::Parenthese(parenthese) => {
            receiver_lookup_expression_is_inert(parenthese.inner_expression())
        }
        Expression::TypeCast(type_cast) => {
            receiver_lookup_expression_is_inert(type_cast.get_expression())
        }
        _ => false,
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
    use crate::rules::test_support::{analyze, generate, parse, parse_lua52};

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
    fn inlines_an_assignment_inside_a_repeat_body_that_runs_once() {
        let output = inlined_output(
            "local f\nrepeat\n    f = function()\n        return 1\n    end\nuntil true\nreturn f()",
        );
        assert!(!output.contains("f()"), "{output}");
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
    fn inlines_a_call_in_a_while_condition() {
        let output = inlined_output(
            "local function check(x)\n    return x\nend\nwhile check(true) do\n    break\nend\nreturn 0",
        );
        assert!(!output.contains("check(true)"), "{output}");
        assert!(output.contains("while true"), "{output}");
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
    fn inlines_an_assignment_bound_call_after_a_goto() {
        let output = inlined_output(
            "local f\nf = function()\n    return 1\nend\ngoto done\n::done::\nreturn f()",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    fn inlined_dialect(code: &str, dialect: crate::Dialect) -> String {
        let (mut block, model) = crate::Parser::default()
            .dialect(dialect)
            .parse_analyzed(code)
            .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
        inline_local_functions(&mut block, &model).expect("the analyzed tree has block identities");
        generate(&block)
    }

    #[test]
    fn inlines_a_method_argument_after_the_puc_lookup() {
        let output = inlined_dialect(
            "local function arg()\n    return 1\nend\nlocal obj = { m = print }\nobj:m(arg())",
            crate::Dialect::Lua53,
        );
        assert!(!output.contains("m(arg())"), "{output}");
        assert!(output.contains("obj"), "{output}");
    }

    #[test]
    fn inlines_a_method_argument_before_the_luau_lookup() {
        let output = inlined_dialect(
            "local function arg()\n    return 1\nend\nlocal obj = { m = print }\nobj:m(arg())",
            crate::Dialect::Luau,
        );
        assert!(!output.contains("m(arg())"), "{output}");
        assert!(output.contains(":m"), "{output}");
    }

    #[test]
    fn keeps_a_method_argument_with_an_unknown_lookup_order() {
        let output = inlined_output(
            "local function arg()\n    return 1\nend\nlocal obj = { m = print }\nobj:m(arg())",
        );
        assert!(output.contains("m(arg())"), "{output}");
    }

    #[test]
    fn inlines_a_method_argument_on_a_fresh_table_in_an_unknown_dialect() {
        let output =
            inlined_output("local function arg()\n    return 1\nend\n({ m = print }):m(arg())");
        assert!(!output.contains("m(arg())"), "{output}");
    }

    #[test]
    fn inlines_when_every_reaching_definition_has_the_same_body() {
        let output = inlined_output(
            "local f\nif math.random() > 0.5 then\n    f = function(x)\n        return x + 1\n    end\nelse\n    f = function(x)\n        return x + 1\n    end\nend\nreturn f(1)",
        );
        assert!(!output.contains("f(1)"), "{output}");
    }

    #[test]
    fn keeps_a_call_when_reaching_definitions_have_different_bodies() {
        let output = inlined_output(
            "local f\nif math.random() > 0.5 then\n    f = function()\n        return 1\n    end\nelse\n    f = function()\n        return 2\n    end\nend\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn keeps_a_call_when_one_path_leaves_the_local_nil() {
        let output = inlined_output(
            "local f\nif math.random() > 0.5 then\n    f = function()\n        return 1\n    end\nend\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_the_last_of_several_assignments() {
        let output = inlined_output(
            "local f\nf = function()\n    return 1\nend\nf = function()\n    return 2\nend\nreturn f()",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_an_alias_that_captured_the_value_before_a_reassignment() {
        let output = inlined_output(
            "local f = function()\n    return 1\nend\nlocal g = f\nf = function()\n    return 2\nend\nreturn g()",
        );
        assert!(!output.contains("g()"), "{output}");
    }

    #[test]
    fn inlines_a_call_after_an_assignment_in_the_same_loop_iteration() {
        let output = inlined_output(
            "for i = 1, 3 do\n    local f\n    f = function()\n        return i\n    end\n    print(f())\nend\nreturn 0",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn keeps_a_call_when_a_nested_function_can_rewrite_the_local() {
        let output = inlined_output(
            "local f = function()\n    return 1\nend\nlocal function reset()\n    f = function()\n        return 2\n    end\nend\nreset()\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn keeps_a_call_when_a_goto_can_skip_the_assignment() {
        let output = inlined_dialect(
            "local f\nif flag then goto after end\nf = function()\n    return 1\nend\n::after::\nreturn f()",
            crate::Dialect::Lua52,
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_a_closure_call_created_after_the_assignment() {
        let output = inlined_output(
            "local f\nif math.random() > 0.5 then\n    f = function(x)\n        return x + 1\n    end\nelse\n    f = function(x)\n        return x + 1\n    end\nend\nlocal g = function()\n    return f(1)\nend\nreturn g()",
        );
        assert!(!output.contains("f(1)"), "{output}");
    }

    #[test]
    fn keeps_a_closure_call_when_a_later_write_can_run() {
        let output = inlined_output(
            "local f = function()\n    return 1\nend\nlocal g = function()\n    return f()\nend\nf = function()\n    return 2\nend\nreturn g()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn keeps_a_closure_call_when_a_loop_can_rewrite_the_upvalue() {
        let output = inlined_output(
            "local f\nfor i = 1, 2 do\n    f = function()\n        return i\n    end\n    local g = function()\n        return f()\n    end\n    print(g())\nend\nreturn 0",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_a_while_condition_without_capturing_a_body_local() {
        let output = inlined_output(
            "local value = 0\nlocal function check()\n    value = value + 1\n    return value < 2\nend\nwhile check() do\n    local value = 10\n    print(value)\nend\nreturn value",
        );
        assert!(!output.contains("while check"), "{output}");
        assert!(output.contains("while true"), "{output}");
    }

    #[test]
    fn keeps_a_conditional_while_condition_operand() {
        let output = inlined_output(
            "local function check()\n    return true\nend\nwhile false and check() do\n    break\nend\nreturn 0",
        );
        assert!(output.contains("check()"), "{output}");
    }

    #[test]
    fn inlines_a_repeat_condition_with_a_body_local() {
        let output = inlined_output(
            "local i = 0\nlocal function check(value)\n    return value >= 2\nend\nrepeat\n    i = i + 1\n    local done = i\nuntil check(done)\nreturn i",
        );
        assert!(!output.contains("until check"), "{output}");
    }

    #[test]
    fn inlines_a_luau_repeat_condition_across_continue() {
        let output = inlined_dialect(
            "local i = 0\nlocal function check()\n    return i >= 3\nend\nrepeat\n    i = i + 1\n    if i == 2 then continue end\n    print(i)\nuntil check()\nreturn i",
            crate::Dialect::Luau,
        );
        assert!(!output.contains("until check"), "{output}");
        assert!(!output.contains("continue"), "{output}");
    }

    #[test]
    fn inlines_a_luau_repeat_condition_that_reads_a_local_before_continue() {
        let output = inlined_dialect(
            "local i = 0\nlocal function check(value)\n    return value >= 3\nend\nrepeat\n    i = i + 1\n    local value = i\n    if i == 2 then continue end\n    print(value)\nuntil check(value)\nreturn i",
            crate::Dialect::Luau,
        );
        assert!(!output.contains("until check"), "{output}");
        assert!(!output.contains("continue"), "{output}");
        assert!(output.contains("local"), "{output}");
    }

    #[test]
    fn inlines_a_repeat_condition_after_a_nested_block_end_label() {
        let code = "local function check()\n    return true\nend\nrepeat\n    do\n        goto continue\n        local skipped = 1\n        ::continue::\n    end\nuntil check()\nreturn 0";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        assert!(!output.contains("until check"), "{output}");
        parse_lua52(&output);
    }

    #[test]
    fn inlines_a_repeat_condition_before_a_body_level_continue_label() {
        let code = "local function check()\n    return true\nend\nrepeat\n    goto continue\n    ::continue::\nuntil check()\nreturn 0";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        assert!(!output.contains("until check"), "{output}");
        parse_lua52(&output);
    }

    #[test]
    fn keeps_an_unreachable_repeat_condition_after_return() {
        let output = inlined_output(
            "local function check()\n    return true\nend\nrepeat\n    return 1\nuntil check()",
        );
        assert!(output.contains("check()"), "{output}");
    }

    #[test]
    fn inlining_between_a_goto_and_its_label_still_reparses() {
        let code = "local function value(x)\n    return x\nend\ngoto done\nprint(value(1))\n::done::\nprint(0)\nreturn 0";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        parse_lua52(&output);
    }

    #[test]
    fn inlining_a_reachable_call_hoists_its_result_in_a_labelled_function() {
        let code = "local function value(x)\n    return x\nend\nlocal first = value(1)\ngoto done\nprint(first)\n::done::\nprint(first)\nreturn first";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        assert!(!output.contains("value(1)"), "{output}");
        parse_lua52(&output);
    }

    #[test]
    fn renames_a_callee_label_that_collides_with_a_caller_label() {
        let code = "local function value()\n    goto done\n    ::done::\n    return 1\nend\n::done::\nprint(value())\nreturn 0";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        assert!(!output.contains("print(value())"), "{output}");
        parse_lua52(&output);
    }

    #[test]
    fn inlines_a_call_in_a_closure_nested_through_another_function() {
        let output = inlined_output(
            "local f\nf = function()\n    return 1\nend\nlocal outer = function()\n    local inner = function()\n        return f()\n    end\n    return inner()\nend\nreturn outer()",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn inlining_a_callee_with_a_forward_goto_reanalyzes() {
        let code = "local function pick(value)\n    if value > 0 then\n        goto positive\n    end\n    do\n        return -1\n    end\n    ::positive::\n    return 1\nend\nprint(pick(1), pick(-1))\nreturn pick(2)";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        crate::Parser::default()
            .lua52()
            .parse_analyzed(&output)
            .unwrap_or_else(|error| panic!("failed to re-analyze:\\n{error}\\n{output}"));
    }

    #[test]
    fn inlines_after_a_later_assignment_overrides_conditional_definitions() {
        let output = inlined_output(
            "local f\nif math.random() > 0.5 then\n    f = function()\n        return 1\n    end\nelse\n    f = function()\n        return 2\n    end\nend\nf = function()\n    return 3\nend\nreturn f()",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_when_a_conditional_keeps_an_identical_previous_value() {
        let output = inlined_output(
            "local f = function()\n    return 1\nend\nif math.random() > 0.5 then\n    f = function()\n        return 1\n    end\nend\nreturn f()",
        );
        assert!(!output.contains("f()"), "{output}");
    }

    #[test]
    fn keeps_a_call_when_a_conditional_keeps_a_different_previous_value() {
        let output = inlined_output(
            "local f = function()\n    return 1\nend\nif math.random() > 0.5 then\n    f = function()\n        return 2\n    end\nend\nreturn f()",
        );
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_a_function_statement_assignment() {
        let output = inlined_output("local f\nfunction f()\n    return 1\nend\nreturn f()");
        assert!(!output.contains("return f()"), "{output}");
    }

    #[test]
    fn inlines_multiple_function_literals_from_one_declaration() {
        let output = inlined_output(
            "local f, g = function()\n    return 1\nend, function()\n    return 2\nend\nreturn f(), g()",
        );
        assert!(!output.contains("f()"), "{output}");
        assert!(!output.contains("g()"), "{output}");
    }

    #[test]
    fn inlines_a_conditional_alias_that_always_names_the_same_function() {
        let output = inlined_output(
            "local f = function()\n    return 1\nend\nlocal g\nif math.random() > 0.5 then\n    g = f\nelse\n    g = f\nend\nreturn g()",
        );
        assert!(!output.contains("g()"), "{output}");
    }

    #[test]
    fn inlines_after_a_backward_goto_that_keeps_the_same_assignment() {
        let code = "local f\nlocal n = 0\n::again::\nn = n + 1\nf = function()\n    return 1\nend\nif n < 1 then\n    goto again\nend\nreturn f()";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        assert!(!output.contains("f()"), "{output}");
        crate::Parser::default()
            .lua52()
            .parse_analyzed(&output)
            .unwrap_or_else(|error| panic!("failed to re-analyze:\\n{error}\\n{output}"));
    }

    #[test]
    fn keeps_a_call_when_a_backward_goto_merges_different_definitions() {
        let code = "local f\nlocal n = 0\n::again::\nn = n + 1\nif n == 1 then\n    f = function()\n        return 1\n    end\nelse\n    f = function()\n        return 2\n    end\nend\nif n < 2 then\n    goto again\nend\nreturn f()";
        let output = inlined_dialect(code, crate::Dialect::Lua52);
        assert!(output.contains("f()"), "{output}");
    }

    #[test]
    fn inlines_a_method_call_whose_receiver_is_a_call() {
        let output = inlined_dialect(
            "local function make()\n    return { m = print }\nend\nlocal function arg()\n    return 1\nend\nmake():m(arg())",
            crate::Dialect::Lua53,
        );
        assert!(!output.contains("m(arg())"), "{output}");
    }

    #[test]
    fn inlines_the_unconditional_while_condition_operand() {
        let output = inlined_output(
            "local function first()\n    return true\nend\nlocal function second()\n    return false\nend\nwhile first() and second() do\n    break\nend\nreturn 0",
        );
        assert!(!output.contains("while first"), "{output}");
        assert!(output.contains("second()"), "{output}");
    }

    #[test]
    fn inlines_a_repeat_condition_calling_a_body_local_function() {
        let output = inlined_output(
            "local i = 0\nrepeat\n    i = i + 1\n    local function check(value)\n        return value >= 2\n    end\n    local current = i\nuntil check(current)\nreturn i",
        );
        assert!(!output.contains("until check"), "{output}");
    }

    #[test]
    fn inlines_a_luau_repeat_condition_calling_a_body_local_function_across_continue() {
        let output = inlined_dialect(
            "local i = 0\nrepeat\n    i = i + 1\n    local function check(value)\n        return value >= 3\n    end\n    local current = i\n    if i == 2 then continue end\n    print(current)\nuntil check(current)\nreturn i",
            crate::Dialect::Luau,
        );
        assert!(!output.contains("until check"), "{output}");
        assert!(!output.contains("continue"), "{output}");
    }

    #[test]
    fn keeps_a_repeat_condition_that_mixes_continue_and_labels() {
        let output = inlined_output(
            "local function check()\n    return true\nend\nrepeat\n    local x = 1\n    if x == 1 then continue end\n    goto done\n    ::done::\nuntil check()\nreturn 0",
        );
        assert!(output.contains("until check()"), "{output}");
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
