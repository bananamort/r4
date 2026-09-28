//! Inlining of calls to local functions.
//!
//! [`inline_local_functions`] replaces a call to an immutable, uncaptured local
//! function with the function's body bound to the arguments at the call site.
//! The function is left in place;
//! [`remove_unused_variables`](super::remove_unused_variables) removes it once
//! its last call has gone.
//!
//! A call is rewritten into a statement-level preamble that runs where the call
//! did, and the call expression becomes the result variable:
//!
//! ```lua
//! local __r                              -- the call's result
//! do
//!     local p1, p2, __d = <arg 1>, <arg 2>, <extra arg>
//!     repeat
//!         <body, with `return e` rewritten to `__r = e; break`>
//!     until true
//! end
//! ```
//!
//! The `repeat ... until true` runs the body once and gives each `return` a
//! `break` to leave through. The parameters are declared with their original
//! names inside the `do`, so the body's references resolve to the arguments
//! exactly as they did to the parameters, and the body's own locals are scoped
//! to the `do` block.
//!
//! Soundness conditions, each checked before a call is rewritten:
//!
//! * the callee is an immutable, uncaptured local function that is not variadic;
//! * the body reads only its parameters, its own locals and globals — never an
//!   upvalue, which is what makes recursive and mutually recursive inlining
//!   impossible (a callee can only see itself, or another local, as an upvalue);
//! * the body contains no `...` and no `goto`/label (a label is function-scoped
//!   and could collide with one at the call site);
//! * the body cannot fall through: its last statement is a `return`;
//! * no `return` is nested inside a loop of the body, because there the injected
//!   `break` would leave that loop instead of the inlined call;
//! * every `return` yields exactly one value, and none of them is itself
//!   multi-valued, so the call has the single result the replacement names.
//!
//! Calls whose callee or body fails any of these are left untouched.

use std::collections::HashMap;

use crate::nodes::{
    AssignStatement, Block, DoStatement, Expression, FunctionAssignment, Identifier, LastStatement,
    Prefix, RepeatStatement, Statement, TypedIdentifier, Variable, VariableAssignment,
};
use crate::process::{
    DefaultPostVisitor, DefaultVisitor, NodePostProcessor, NodePostVisitor, NodeProcessor,
    NodeVisitor,
};
use crate::rules::NameGenerator;
use crate::semantic::{ScopeId, SemanticModel, SymbolId, SymbolKind};

/// A local function that can be inlined at its call sites.
struct InlinableFunction {
    /// The parameter names, in declaration order.
    parameters: Vec<String>,
    /// The body, which reads only the parameters, its own locals and globals.
    body: Block,
}

/// Replaces calls to inlinable local functions by their body.
///
/// Returns the number of calls inlined. See the module documentation for the
/// conditions a call and its callee must satisfy.
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
    let mut inliner = BlockInliner {
        model,
        functions: &collector.functions,
        names: &mut names,
        inlined: &mut inlined,
    };
    DefaultVisitor::visit_block(block, &mut inliner);

    inlined
}

// ----------------------------------------------------------------------
// Deciding what can be inlined
// ----------------------------------------------------------------------

/// Returns whether a local function can be inlined, and its shape.
fn inlinable(
    model: &SemanticModel,
    symbol: SymbolId,
    body: &Block,
    parameters: &[&TypedIdentifier],
    variadic: bool,
) -> Option<InlinableFunction> {
    if variadic {
        return None;
    }

    let info = model.symbol(symbol);
    if info.kind != SymbolKind::Local || info.is_captured() || !model.is_immutable(symbol) {
        return None;
    }

    // The body must not fall through: control reaching the end runs this.
    let Some(LastStatement::Return(_)) = body.get_last_statement() else {
        return None;
    };

    let mut check = BodyCheck {
        model,
        scope: model.try_scope_of(body).ok()?,
        loop_depth: 0,
        function_depth: 0,
        rejects: false,
    };
    check.visit(body);

    if check.rejects {
        return None;
    }

    Some(InlinableFunction {
        parameters: parameters
            .iter()
            .map(|parameter| parameter.get_identifier().get_name().clone())
            .collect(),
        body: body.clone(),
    })
}

/// Verifies that a body reads nothing it must not, and lowerable returns.
struct BodyCheck<'m> {
    model: &'m SemanticModel,
    /// The function's own scope; a symbol declared outside it is an upvalue.
    scope: ScopeId,
    loop_depth: usize,
    /// Zero while inside the function whose calls are being inlined.
    function_depth: usize,
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
            Statement::While(statement) => self.visit_loop(statement.get_block()),
            Statement::Repeat(statement) => self.visit_loop(statement.get_block()),
            Statement::NumericFor(statement) => self.visit_loop(statement.get_block()),
            Statement::GenericFor(statement) => self.visit_loop(statement.get_block()),
            Statement::Do(statement) => self.visit(statement.get_block()),
            Statement::Function(statement) => self.visit_function(statement.get_block()),
            Statement::LocalFunction(statement) => self.visit_function(statement.get_block()),
            Statement::If(statement) => {
                for branch in statement.iter_branches() {
                    self.visit(branch.get_block());
                }
                if let Some(else_block) = statement.get_else_block() {
                    self.visit(else_block);
                }
            }
            Statement::Goto(_) | Statement::Label(_) => self.rejects = true,
            _ => {}
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
            // A nested function's `return` belongs to that function.
            return;
        }
        if self.loop_depth != 0 {
            // The injected `break` would leave the body's loop, not the call.
            self.rejects = true;
            return;
        }

        let expressions: Vec<&Expression> = return_statement.iter_expressions().collect();
        if expressions.len() != 1
            || matches!(
                expressions[0],
                Expression::Call(_) | Expression::VariableArguments(_)
            )
        {
            // The call would not have exactly one result value.
            self.rejects = true;
            return;
        }
        self.visit_expression(expressions[0]);
    }

    fn visit_expression(&mut self, expression: &Expression) {
        match expression {
            Expression::Identifier(identifier) => {
                let Some(symbol) = self.model.resolve_identifier(identifier) else {
                    self.rejects = true;
                    return;
                };
                let declaring = self.model.symbol(symbol).declaring_scope;
                if self.model.symbol(symbol).kind == SymbolKind::Global {
                    return;
                }
                let mut scope = Some(declaring);
                while let Some(id) = scope {
                    if id == self.scope {
                        return;
                    }
                    scope = self.model.scope(id).parent;
                }
                // Declared outside the function: an upvalue.
                self.rejects = true;
            }
            Expression::VariableArguments(_) => self.rejects = true,
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
            }
            _ => {}
        }
    }
}

/// Collects every inlinable local function in a block.
struct FunctionCollector<'m> {
    model: &'m SemanticModel,
    functions: HashMap<SymbolId, InlinableFunction>,
}

impl FunctionCollector<'_> {
    fn record(
        &mut self,
        identifier: &Identifier,
        body: &Block,
        parameters: &[&TypedIdentifier],
        variadic: bool,
    ) {
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return;
        };
        if let Some(function) = inlinable(self.model, symbol, body, parameters, variadic) {
            self.functions.insert(symbol, function);
        }
    }
}

impl NodeProcessor for FunctionCollector<'_> {
    fn process_local_function_statement(&mut self, statement: &mut FunctionAssignment) {
        let parameters: Vec<&TypedIdentifier> = statement.iter_parameters().collect();
        let (identifier, body, variadic) = (
            statement.get_identifier(),
            statement.get_block(),
            statement.is_variadic(),
        );
        self.record(identifier, body, &parameters, variadic);
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
        let (identifier, body, variadic) = (
            variable.get_identifier(),
            function.get_block(),
            function.is_variadic(),
        );
        self.record(identifier, body, &parameters, variadic);
    }
}

// ----------------------------------------------------------------------
// Rewriting
// ----------------------------------------------------------------------

/// Rewrites every block, inserting a call's preamble before its statement.
struct BlockInliner<'m, 'n, 'i> {
    model: &'m SemanticModel,
    functions: &'m HashMap<SymbolId, InlinableFunction>,
    names: &'n mut NameGenerator,
    inlined: &'i mut usize,
}

impl NodeProcessor for BlockInliner<'_, '_, '_> {
    fn process_block(&mut self, block: &mut Block) {
        let statements = block.take_statements();
        let mut rewritten = Vec::with_capacity(statements.len());
        for mut statement in statements {
            let mut preambles = Vec::new();
            {
                let mut inliner = StatementInliner {
                    model: self.model,
                    functions: self.functions,
                    names: self.names,
                    block_depth: 0,
                    preambles: &mut preambles,
                    inlined: self.inlined,
                };
                DefaultPostVisitor::visit_statement(&mut statement, &mut inliner);
            }
            rewritten.append(&mut preambles);
            rewritten.push(statement);
        }
        block.set_statements(rewritten);

        if let Some(mut last) = block.get_last_statement().cloned() {
            let mut preambles = Vec::new();
            {
                let mut inliner = StatementInliner {
                    model: self.model,
                    functions: self.functions,
                    names: self.names,
                    block_depth: 0,
                    preambles: &mut preambles,
                    inlined: self.inlined,
                };
                DefaultPostVisitor::visit_last_statement(&mut last, &mut inliner);
            }
            if !preambles.is_empty() {
                let mut statements = block.take_statements();
                statements.append(&mut preambles);
                block.set_statements(statements);
            }
            block.set_last_statement(last);
        }
    }
}

/// Finds inlinable calls in one statement, stopping at nested blocks.
struct StatementInliner<'m, 'n, 'p, 'i> {
    model: &'m SemanticModel,
    functions: &'m HashMap<SymbolId, InlinableFunction>,
    names: &'n mut NameGenerator,
    block_depth: usize,
    preambles: &'p mut Vec<Statement>,
    inlined: &'i mut usize,
}

impl StatementInliner<'_, '_, '_, '_> {
    fn inline_call(&mut self, call: &crate::nodes::FunctionCall) -> Option<Expression> {
        if call.get_method().is_some() {
            return None;
        }
        let Prefix::Identifier(identifier) = call.get_prefix() else {
            return None;
        };
        let symbol = self.model.resolve_identifier(identifier)?;
        let function = self.functions.get(&symbol)?;

        let result = self.names.generate();

        // Bind the arguments to the parameters exactly as the call did; extra
        // arguments are evaluated into fresh, unused locals.
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
        lower_returns(&mut body, &result);

        let inner = Block::new(
            vec![
                VariableAssignment::new(bindings, values).into(),
                RepeatStatement::new(body, Expression::from(true)).into(),
            ],
            None,
        );

        self.preambles.push(
            VariableAssignment::new(vec![TypedIdentifier::new(result.clone())], Vec::new()).into(),
        );
        self.preambles.push(DoStatement::new(inner).into());
        *self.inlined += 1;

        Some(Expression::identifier(result))
    }
}

impl NodeProcessor for StatementInliner<'_, '_, '_, '_> {
    fn process_block(&mut self, _block: &mut Block) {
        self.block_depth += 1;
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        if self.block_depth != 0 {
            return;
        }
        let Expression::Call(call) = expression else {
            return;
        };
        if let Some(replacement) = self.inline_call(call) {
            *expression = replacement;
        }
    }
}

impl NodePostProcessor for StatementInliner<'_, '_, '_, '_> {
    fn process_after_block(&mut self, _block: &mut Block) {
        self.block_depth -= 1;
    }
}

/// Rewrites every `return` of a body into `result = <expression>` and a `break`.
fn lower_returns(block: &mut Block, result: &str) {
    lower_block_returns(block, result, 0);
}

fn lower_block_returns(block: &mut Block, result: &str, depth: usize) {
    let mut statements = block.take_statements();
    for statement in statements.iter_mut() {
        lower_statement_returns(statement, result, depth);
    }
    block.set_statements(statements);

    let Some(last) = block.get_last_statement().cloned() else {
        return;
    };
    match last {
        LastStatement::Return(return_statement) if depth == 0 => {
            let values: Vec<Expression> = return_statement.iter_expressions().cloned().collect();
            let mut statements = block.take_statements();
            statements.push(
                AssignStatement::new(
                    vec![Variable::Identifier(Identifier::new(result.to_owned()))],
                    values,
                )
                .into(),
            );
            block.set_statements(statements);
            block.set_last_statement(LastStatement::Break(None));
        }
        LastStatement::Return(_) => {}
        other => {
            // `break`/`continue` have no nested blocks.
            let _ = other;
        }
    }
}

fn lower_statement_returns(statement: &mut Statement, result: &str, depth: usize) {
    match statement {
        Statement::Do(statement) => lower_block_returns(statement.mutate_block(), result, depth),
        Statement::If(statement) => {
            for branch in statement.mutate_branches() {
                lower_block_returns(branch.mutate_block(), result, depth);
            }
            if let Some(else_block) = statement.mutate_else_block() {
                lower_block_returns(else_block, result, depth);
            }
        }
        Statement::While(statement) => lower_block_returns(statement.mutate_block(), result, depth),
        Statement::Repeat(statement) => {
            lower_block_returns(statement.mutate_block(), result, depth)
        }
        Statement::NumericFor(statement) => {
            lower_block_returns(statement.mutate_block(), result, depth)
        }
        Statement::GenericFor(statement) => {
            lower_block_returns(statement.mutate_block(), result, depth)
        }
        Statement::Function(statement) => {
            lower_block_returns(statement.mutate_block(), result, depth + 1)
        }
        Statement::LocalFunction(statement) => {
            lower_block_returns(statement.mutate_block(), result, depth + 1)
        }
        Statement::TypeFunction(statement) => {
            lower_block_returns(statement.mutate_block(), result, depth + 1)
        }
        _ => {}
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate, parse};

    #[test]
    fn inlines_a_body_with_statements() {
        let (mut block, model) = analyze(
            "local function add(a, b)\n    local sum = a + b\n    return sum\nend\nreturn add(1, 2)",
        );

        let inlined = inline_local_functions(&mut block, &model);

        assert_eq!(inlined, 1);
        let output = generate(&block);
        assert!(!output.contains("add(1, 2)"), "{output}");
        assert!(output.contains("local sum = a + b"), "{output}");
    }

    #[test]
    fn inlines_a_body_with_control_flow() {
        let (mut block, model) = analyze(
            "local function sign(value)\n    if value > 0 then\n        return 1\n    end\n    return -1\nend\nreturn sign(3)",
        );

        let inlined = inline_local_functions(&mut block, &model);

        assert_eq!(inlined, 1);
        assert!(!generate(&block).contains("sign(3)"));
        assert!(generate(&block).contains("return 1"));
    }

    #[test]
    fn inlines_a_body_with_a_loop() {
        let (mut block, model) = analyze(
            "local function sum(n)\n    local total = 0\n    for i = 1, n do\n        total = total + i\n    end\n    return total\nend\nreturn sum(3)",
        );

        assert_eq!(inline_local_functions(&mut block, &model), 1);
    }

    #[test]
    fn evaluates_arguments_once_however_often_a_parameter_is_used() {
        let (mut block, model) = analyze(
            "local function twice(value)\n    return value + value\nend\nreturn twice(bump())",
        );

        let inlined = inline_local_functions(&mut block, &model);

        assert_eq!(inlined, 1);
        let output = generate(&block);
        assert_eq!(output.matches("bump()").count(), 1, "{output}");
    }

    #[test]
    fn does_not_inline_a_return_inside_a_loop() {
        let (mut block, model) = analyze(
            "local function find(n)\n    for i = 1, n do\n        if i == 2 then\n            return i\n        end\n    end\n    return -1\nend\nreturn find(5)",
        );

        assert_eq!(inline_local_functions(&mut block, &model), 0);
    }

    #[test]
    fn does_not_inline_a_body_that_falls_through() {
        let (mut block, model) =
            analyze("local function show(x)\n    print(x)\nend\nreturn show(1)");

        assert_eq!(inline_local_functions(&mut block, &model), 0);
    }

    #[test]
    fn does_not_inline_an_ambiguous_return_arity() {
        let (mut block, model) =
            analyze("local function pair(x)\n    return x, x\nend\nreturn pair(1)");

        assert_eq!(inline_local_functions(&mut block, &model), 0);
    }

    #[test]
    fn does_not_inline_a_recursive_function() {
        let (mut block, model) = analyze("local function f(x)\n    return f(x)\nend\nreturn f(1)");

        assert_eq!(inline_local_functions(&mut block, &model), 0);
    }

    #[test]
    fn does_not_inline_an_upvalue_reference() {
        let (mut block, model) =
            analyze("local y = 2\nlocal function f(x)\n    return x + y\nend\nreturn f(1)");

        assert_eq!(inline_local_functions(&mut block, &model), 0);
    }

    #[test]
    fn the_inlined_output_reparses() {
        for code in [
            "local function add(a, b)\n    local sum = a + b\n    return sum\nend\nreturn add(1, 2)",
            "local function sign(value)\n    if value > 0 then\n        return 1\n    end\n    return -1\nend\nreturn sign(3)",
            "local function sum(n)\n    local total = 0\n    for i = 1, n do\n        total = total + i\n    end\n    return total\nend\nreturn sum(3)",
            "local function show(x)\n    print(x)\n    return x\nend\nprint(show(1))",
            "local function f(a, b)\n    return a\nend\nreturn f(1, 2, 3)",
        ] {
            let (mut block, model) = analyze(code);
            inline_local_functions(&mut block, &model);
            parse(&generate(&block));
        }
    }
}
