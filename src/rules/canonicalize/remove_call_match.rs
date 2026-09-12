use std::collections::HashMap;

use crate::nodes::{
    Expression, FunctionCall, Identifier, Statement, TypedIdentifier, VariableAssignment,
};
use crate::process::{DefaultVisitor, Evaluator, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolKind};
use crate::utils::{
    expressions_as_expression, expressions_as_statement, preserve_arguments_side_effects,
};

use crate::rules::name_generator::NameGenerator;

/// Decides which calls a [`remove_call_matches`] pass removes and what replaces
/// them.
pub trait CallMatcher {
    /// Returns whether `call` targets the function this matcher removes.
    ///
    /// Matchers must resolve the call prefix through the model (see
    /// [`identifier_resolves_to_global`]); a prefix that cannot be resolved
    /// must not match.
    fn matches(&self, call: &FunctionCall, model: &SemanticModel) -> bool;

    /// Computes the replacement expression. `None` means the call result is
    /// dropped; argument side effects are preserved either way.
    fn compute_result(
        &self,
        call: &FunctionCall,
        reserved: &HashMap<&'static str, String>,
    ) -> Option<Expression>;

    /// Global functions that [`Self::compute_result`] may reference.
    fn reserve_globals(&self) -> &'static [&'static str] {
        &[]
    }

    /// Returns whether this specific call requires the reserved globals. The
    /// default is `true` whenever any global is declared, so matchers whose
    /// result only needs them for some calls can avoid unused aliases.
    fn needs_reserved_globals(&self, _call: &FunctionCall) -> bool {
        !self.reserve_globals().is_empty()
    }
}

/// Returns whether `identifier` refers to the global function `name`, either
/// directly or through a read-only alias.
///
/// This is the strict check matchers should use: an identifier that cannot be
/// resolved, a mutable local, or a local that does not alias the global all
/// return `false`.
pub fn identifier_resolves_to_global(
    model: &SemanticModel,
    identifier: &Identifier,
    name: &str,
) -> bool {
    let Some(symbol) = model.resolve_identifier(identifier) else {
        return false;
    };

    let info = model.symbol(symbol);
    if info.kind == SymbolKind::Global {
        return info.name == name;
    }

    match model.immutable_global_alias(symbol) {
        Some(global) => model.symbol(global).name == name,
        None => false,
    }
}

/// Removes every call matched by `matcher`, preserving argument side effects.
///
/// Returns the number of calls removed. When a reserved global is shadowed, a
/// collision-free alias is allocated with the model-first [`NameGenerator`] and
/// inserted at the top of `block`; otherwise the global name is used directly.
pub fn remove_call_matches<M: CallMatcher>(
    block: &mut crate::nodes::Block,
    model: &SemanticModel,
    matcher: &M,
) -> usize {
    let mut processor = Processor {
        model,
        matcher,
        evaluator: Evaluator::default(),
        names: NameGenerator::new(model),
        reserved: HashMap::new(),
        aliases: Vec::new(),
        removed: 0,
    };
    DefaultVisitor::visit_block(block, &mut processor);

    if !processor.aliases.is_empty() {
        let mut aliases = processor.aliases;
        aliases.sort_by_key(|(global, _)| *global);

        let variables: Vec<TypedIdentifier> = aliases
            .iter()
            .map(|(_, name)| TypedIdentifier::new(name.clone()))
            .collect();
        let values: Vec<Expression> = aliases
            .iter()
            .map(|(global, _)| Identifier::new((*global).to_owned()).into())
            .collect();

        block.insert_statement(0, VariableAssignment::new(variables, values));
    }

    processor.removed
}

struct Processor<'a, M> {
    model: &'a SemanticModel,
    matcher: &'a M,
    evaluator: Evaluator,
    names: NameGenerator,
    reserved: HashMap<&'static str, String>,
    aliases: Vec<(&'static str, String)>,
    removed: usize,
}

impl<M: CallMatcher> Processor<'_, M> {
    /// Returns the name to use for `global`, generating a collision-free alias
    /// when the global is shadowed.
    fn reserve(&mut self, global: &'static str) -> String {
        if let Some(name) = self.reserved.get(global) {
            return name.clone();
        }

        let name = if self.model.name_is_transparent_global(global) {
            global.to_owned()
        } else {
            let alias = self
                .names
                .generate_with_prefix(format!("__R4_{}", global.to_ascii_uppercase()));
            self.aliases.push((global, alias.clone()));
            alias
        };

        self.reserved.insert(global, name.clone());
        name
    }

    fn matching_call(&self, call: &FunctionCall) -> bool {
        call.get_method().is_none() && self.matcher.matches(call, self.model)
    }
}

impl<M: CallMatcher> NodeProcessor for Processor<'_, M> {
    fn process_statement(&mut self, statement: &mut Statement) {
        let Statement::Call(call) = statement else {
            return;
        };
        if !self.matching_call(call) {
            return;
        }

        let values = preserve_arguments_side_effects(&self.evaluator, call.get_arguments());
        *statement = expressions_as_statement(values);
        self.removed += 1;
    }

    fn process_expression(&mut self, expression: &mut Expression) {
        let Expression::Call(call) = expression else {
            return;
        };
        if !self.matching_call(call) {
            return;
        }

        if self.matcher.needs_reserved_globals(call) {
            let globals: Vec<&'static str> = self.matcher.reserve_globals().to_vec();
            for global in globals {
                self.reserve(global);
            }
        }

        let replacement = self.matcher.compute_result(call, &self.reserved);
        *expression = match replacement {
            Some(result) => result,
            None => expressions_as_expression(preserve_arguments_side_effects(
                &self.evaluator,
                call.get_arguments(),
            )),
        };
        self.removed += 1;
    }
}
