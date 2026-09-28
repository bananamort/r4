//! Inlining of calls to small local functions.
//!
//! [`inline_local_functions`] replaces a call to a local function whose body is
//! a single `return <expression>` with that expression, substituting each
//! parameter by the argument at the call site. The function itself is left in
//! place; [`remove_unused_variables`](super::remove_unused_variables) removes it
//! afterwards when the last call has gone.
//!
//! The rewrite is only sound when evaluating the substituted expression
//! preserves the call's observable behavior, so it is deliberately narrow:
//!
//! * the callee must resolve to an immutable, uncaptured local function whose
//!   body is exactly one returned expression, with no `...` and no nested
//!   function (which keeps the substitution inside one scope, and makes a
//!   recursive call impossible to inline);
//! * the returned expression may read the parameters and globals, and nothing
//!   else;
//! * every argument must be *unobservable* — no side effect and no possible
//!   error — so evaluating it once, not at all, or in a different order (which
//!   substitution may cause) cannot be observed.
//!
//! When any of those cannot be proven the call is left untouched.

use std::collections::HashMap;

use crate::nodes::{
    Block, Expression, FunctionAssignment, LastStatement, Prefix, TypedIdentifier,
    VariableAssignment,
};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{has_observable_effect, SemanticModel, SymbolId, SymbolKind};

/// A local function that can be inlined at its call sites.
struct InlinableFunction {
    /// The parameter symbols, in declaration order.
    parameters: Vec<SymbolId>,
    /// The returned expression, which reads only the parameters and globals.
    result: Expression,
}

/// Replaces calls to small local functions by their returned expression.
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

    let mut inliner = CallInliner {
        model,
        evaluator: Evaluator::default(),
        functions: &collector.functions,
        inlined: 0,
    };
    DefaultVisitor::visit_block(block, &mut inliner);

    inliner.inlined
}

/// Returns the inlinable description of a local function, if it has one.
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

    // The body must be exactly one `return <expression>`.
    if body.iter_statements().next().is_some() {
        return None;
    }
    let Some(LastStatement::Return(return_statement)) = body.get_last_statement() else {
        return None;
    };
    let mut expressions = return_statement.iter_expressions();
    let result = expressions.next()?.clone();
    if expressions.next().is_some() {
        return None;
    }

    let mut parameter_symbols = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        parameter_symbols.push(model.resolve_identifier(parameter.get_identifier())?);
    }

    // The result must stay within the parameters and globals: that rules out a
    // nested closure (whose scope the substitution could not reason about),
    // `...` (which belongs to a different call), and any recursive reference.
    let mut references = ResultReferences {
        model,
        symbols: Vec::new(),
        has_function: false,
        has_varargs: false,
    };
    let mut result = result;
    DefaultVisitor::visit_expression(&mut result, &mut references);
    if references.has_function || references.has_varargs {
        return None;
    }
    for referenced in references.symbols {
        let kind = model.symbol(referenced).kind;
        if kind != SymbolKind::Global && !parameter_symbols.contains(&referenced) {
            return None;
        }
    }

    Some(InlinableFunction {
        parameters: parameter_symbols,
        result,
    })
}

/// Collects every inlinable local function in a block.
struct FunctionCollector<'m> {
    model: &'m SemanticModel,
    functions: HashMap<SymbolId, InlinableFunction>,
}

impl FunctionCollector<'_> {
    fn record(
        &mut self,
        identifier: &crate::nodes::Identifier,
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
        let (identifier, block, variadic) = (
            statement.get_identifier(),
            statement.get_block(),
            statement.is_variadic(),
        );
        self.record(identifier, block, &parameters, variadic);
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
        let identifier = variable.get_identifier();
        let parameters: Vec<&TypedIdentifier> = function.iter_parameters().collect();
        let (identifier, block, variadic) =
            (identifier, function.get_block(), function.is_variadic());
        self.record(identifier, block, &parameters, variadic);
    }
}

/// Collects the symbols a returned expression reads.
struct ResultReferences<'m> {
    model: &'m SemanticModel,
    symbols: Vec<SymbolId>,
    has_function: bool,
    has_varargs: bool,
}

impl NodeProcessor for ResultReferences<'_> {
    fn process_expression(&mut self, expression: &mut Expression) {
        match expression {
            Expression::Identifier(identifier) => {
                if let Some(symbol) = self.model.resolve_identifier(identifier) {
                    self.symbols.push(symbol);
                }
            }
            Expression::Function(_) => self.has_function = true,
            Expression::VariableArguments(_) => self.has_varargs = true,
            _ => {}
        }
    }
}

/// Replaces calls to the collected functions.
struct CallInliner<'m> {
    model: &'m SemanticModel,
    evaluator: Evaluator,
    functions: &'m HashMap<SymbolId, InlinableFunction>,
    inlined: usize,
}

impl CallInliner<'_> {
    fn inline_call(&self, call: &crate::nodes::FunctionCall) -> Option<Expression> {
        if call.get_method().is_some() {
            return None;
        }
        let Prefix::Identifier(identifier) = call.get_prefix() else {
            return None;
        };
        let symbol = self.model.resolve_identifier(identifier)?;
        let function = self.functions.get(&symbol)?;

        let arguments = call.get_arguments().clone().to_expressions();
        for argument in &arguments {
            if has_observable_effect(self.model, &self.evaluator, argument) {
                return None;
            }
        }

        let mut substitutions: HashMap<SymbolId, Expression> = HashMap::new();
        for (index, parameter) in function.parameters.iter().enumerate() {
            let argument = arguments
                .get(index)
                .cloned()
                .unwrap_or_else(Expression::nil);
            substitutions.insert(*parameter, argument);
        }

        let mut result = function.result.clone();
        let mut substituter = ParameterSubstituter {
            model: self.model,
            substitutions: &substitutions,
        };
        DefaultVisitor::visit_expression(&mut result, &mut substituter);
        Some(result)
    }
}

impl NodeProcessor for CallInliner<'_> {
    fn process_expression(&mut self, expression: &mut Expression) {
        let Expression::Call(call) = expression else {
            return;
        };
        if let Some(replacement) = self.inline_call(call) {
            *expression = replacement;
            self.inlined += 1;
        }
    }
}

/// Replaces reads of an inlined function's parameters with the arguments.
struct ParameterSubstituter<'m> {
    model: &'m SemanticModel,
    substitutions: &'m HashMap<SymbolId, Expression>,
}

impl NodeProcessor for ParameterSubstituter<'_> {
    fn process_expression(&mut self, expression: &mut Expression) {
        let Expression::Identifier(identifier) = expression else {
            return;
        };
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return;
        };
        if let Some(argument) = self.substitutions.get(&symbol) {
            *expression = argument.clone();
        }
    }

    fn process_prefix_expression(&mut self, prefix: &mut Prefix) {
        // A prefix needs a parenthesized value so a literal or table argument
        // stays syntactically valid (`("abc"):upper()`, `({}).field`).
        let Prefix::Identifier(identifier) = prefix else {
            return;
        };
        let Some(symbol) = self.model.resolve_identifier(identifier) else {
            return;
        };
        if let Some(argument) = self.substitutions.get(&symbol) {
            *prefix = argument.clone().in_parentheses().into();
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn inlines_a_single_return_function() {
        let (mut block, model) =
            analyze("local function double(x)\n    return x * 2\nend\nreturn double(3)");

        let inlined = inline_local_functions(&mut block, &model);

        assert_eq!(inlined, 1);
        assert!(generate(&block).contains("return 3 * 2"));
    }

    #[test]
    fn substitutes_every_parameter_and_extra_arguments() {
        let (mut block, model) =
            analyze("local function sum(a, b)\n    return a + b\nend\nreturn sum(1, 2)");

        inline_local_functions(&mut block, &model);

        assert!(generate(&block).contains("return 1 + 2"));
    }

    #[test]
    fn substitutes_a_parenthesized_prefix() {
        let (mut block, model) = analyze(
            "local function value(t)\n    return t.value\nend\nreturn value({ value = 1 })",
        );

        inline_local_functions(&mut block, &model);

        let output = generate(&block);
        assert!(output.contains("({value = 1}).value"), "{output}");
    }

    #[test]
    fn does_not_inline_when_an_argument_is_observable() {
        let (mut block, model) =
            analyze("local function identity(x)\n    return x\nend\nreturn identity(compute())");

        let inlined = inline_local_functions(&mut block, &model);

        assert_eq!(inlined, 0);
        assert!(generate(&block).contains("identity(compute())"));
    }

    #[test]
    fn does_not_inline_a_body_with_statements() {
        let (mut block, model) =
            analyze("local function f(x)\n    local y = x\n    return y\nend\nreturn f(1)");

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
}
