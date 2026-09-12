use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::nodes::{
    FunctionAssignment, FunctionExpression, FunctionStatement, GenericForStatement, Identifier,
    NumericForStatement, TypeFunctionStatement, TypedIdentifier, VariableAssignment,
};
use crate::process::utils::{identifier_permutator, is_valid_identifier};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{AccessKind, SemanticModel, SymbolId, SymbolKind};

use super::globals::{DEFAULT as DEFAULT_GLOBALS, ROBLOX as ROBLOX_GLOBALS};

/// An error produced while renaming variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameError {
    /// A symbol occurrence has no source position, so it could not be rewritten
    /// consistently. Parse with token preservation to run this pass.
    MissingTokenData {
        /// The name of the symbol that could not be fully rewritten.
        name: String,
    },
}

impl fmt::Display for RenameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenameError::MissingTokenData { name } => write!(
                f,
                "symbol `{}` has an occurrence without token data; parse with `preserve_tokens` to rename variables",
                name
            ),
        }
    }
}

impl std::error::Error for RenameError {}

/// Options controlling variable renaming.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameOptions {
    /// Additional names that generated identifiers must avoid.
    pub reserved_names: Vec<String>,
}

/// Renames every non-global symbol to a short generated identifier.
///
/// Symbols are renamed by their [`SymbolId`] bindings, so shadowed locals,
/// parameters, iteration variables and captured upvalues are rewritten
/// consistently regardless of their names. Global symbols keep their names;
/// generated identifiers are globally unique and avoid every global, every
/// original symbol name, the default host globals and Lua keywords.
///
/// The pass resolves occurrences through source tokens, so the AST must be
/// parsed with token preservation. When any usage cannot be addressed the pass
/// fails without modifying the block.
pub fn rename_variables(
    block: &mut crate::nodes::Block,
    model: &SemanticModel,
) -> Result<usize, RenameError> {
    rename_variables_with_options(block, model, &RenameOptions::default())
}

/// Like [`rename_variables`], with explicit options.
pub fn rename_variables_with_options(
    block: &mut crate::nodes::Block,
    model: &SemanticModel,
    options: &RenameOptions,
) -> Result<usize, RenameError> {
    let candidates: Vec<SymbolId> = model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind != SymbolKind::Global)
        .map(|symbol| symbol.id)
        .collect();

    validate_usages(&candidates, model)?;

    let mut reserved: HashSet<String> = HashSet::new();
    reserved.extend(DEFAULT_GLOBALS.iter().map(|name| (*name).to_owned()));
    reserved.extend(ROBLOX_GLOBALS.iter().map(|name| (*name).to_owned()));
    reserved.extend(model.symbols.iter().map(|symbol| symbol.name.clone()));
    reserved.extend(options.reserved_names.iter().cloned());

    let mut permutator = identifier_permutator();
    let mut renames: HashMap<SymbolId, String> = HashMap::new();

    for &symbol in &candidates {
        let name = loop {
            let candidate = permutator
                .next()
                .expect("the identifier permutator always yields names");
            if is_valid_identifier(&candidate) && !reserved.contains(&candidate) {
                reserved.insert(candidate.clone());
                break candidate;
            }
        };
        renames.insert(symbol, name);
    }

    let mut renamer = Renamer {
        model,
        renames: &renames,
    };
    DefaultVisitor::visit_block(block, &mut renamer);

    Ok(candidates.len())
}

fn validate_usages(candidates: &[SymbolId], model: &SemanticModel) -> Result<(), RenameError> {
    for &symbol in candidates {
        for usage in model.uses_of(symbol) {
            // Implicit declarations (the method `self` parameter) have no
            // identifier in the AST, so only non-declaration usages must be
            // addressable by token.
            if usage.kind != AccessKind::Declaration && usage.token.is_none() {
                return Err(RenameError::MissingTokenData {
                    name: model.symbol(symbol).name.clone(),
                });
            }
        }
    }

    Ok(())
}

struct Renamer<'a> {
    model: &'a SemanticModel,
    renames: &'a HashMap<SymbolId, String>,
}

impl Renamer<'_> {
    fn rename_identifier(&self, identifier: &mut Identifier) {
        if let Some(symbol) = self.model.resolve_identifier(identifier) {
            if let Some(name) = self.renames.get(&symbol) {
                identifier.set_name(name.clone());
            }
        }
    }

    fn rename_typed_identifier(&self, identifier: &mut TypedIdentifier) {
        self.rename_identifier(identifier);
    }

    fn rename_parameters<'a>(&self, parameters: impl Iterator<Item = &'a mut TypedIdentifier>) {
        for parameter in parameters {
            self.rename_typed_identifier(parameter);
        }
    }
}

impl NodeProcessor for Renamer<'_> {
    fn process_variable_expression(&mut self, identifier: &mut Identifier) {
        self.rename_identifier(identifier);
    }

    fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
        for variable in assignment.iter_mut_variables() {
            self.rename_typed_identifier(variable);
        }
    }

    fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
        self.rename_identifier(function.mutate_identifier());
        self.rename_parameters(function.iter_mut_parameters());
    }

    fn process_function_statement(&mut self, function: &mut FunctionStatement) {
        self.rename_parameters(function.iter_mut_parameters());
    }

    fn process_function_expression(&mut self, function: &mut FunctionExpression) {
        self.rename_parameters(function.iter_mut_parameters());
    }

    fn process_numeric_for_statement(&mut self, statement: &mut NumericForStatement) {
        self.rename_typed_identifier(statement.mutate_identifier());
    }

    fn process_generic_for_statement(&mut self, statement: &mut GenericForStatement) {
        for identifier in statement.iter_mut_identifiers() {
            self.rename_typed_identifier(identifier);
        }
    }

    fn process_type_function(&mut self, function: &mut TypeFunctionStatement) {
        self.rename_parameters(function.iter_mut_parameters());
    }
}
