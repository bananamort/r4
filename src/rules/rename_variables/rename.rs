use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::nodes::{
    FunctionAssignment, FunctionExpression, FunctionStatement, GenericForStatement, Identifier,
    NumericForStatement, TypeFunctionStatement, TypedIdentifier, VariableAssignment,
};
use crate::process::utils::{identifier_permutator, is_valid_identifier};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{SemanticModel, SymbolId, SymbolKind};

use super::globals::{DEFAULT as DEFAULT_GLOBALS, ROBLOX as ROBLOX_GLOBALS};

/// An error produced while renaming variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameError {
    /// An identifier is not bound in the semantic model, so it cannot be
    /// renamed consistently. This usually means the AST does not match the
    /// analyzed model.
    UnresolvedIdentifier {
        /// The name of the identifier that could not be resolved.
        name: String,
    },
}

impl fmt::Display for RenameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenameError::UnresolvedIdentifier { name } => write!(
                f,
                "identifier `{}` is not bound in the semantic model",
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
/// Identifiers are resolved through the structural bindings recorded by
/// semantic analysis, so the pass works on token-less trees. When any
/// identifier cannot be resolved the pass fails without modifying the block.
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

    validate_identifiers(block, model)?;

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

/// Verifies that every identifier the renamer can rewrite is bound in the
/// model, so a stale or foreign model is rejected before any mutation.
fn validate_identifiers(
    block: &mut crate::nodes::Block,
    model: &SemanticModel,
) -> Result<(), RenameError> {
    struct Validator<'a> {
        model: &'a SemanticModel,
        error: Option<RenameError>,
    }

    impl Validator<'_> {
        fn check(&mut self, identifier: &Identifier) {
            if self.error.is_some() {
                return;
            }
            if self.model.resolve_identifier(identifier).is_none() {
                self.error = Some(RenameError::UnresolvedIdentifier {
                    name: identifier.get_name().clone(),
                });
            }
        }

        fn check_parameters<'a>(&mut self, parameters: impl Iterator<Item = &'a TypedIdentifier>) {
            for parameter in parameters {
                self.check(parameter);
            }
        }
    }

    impl NodeProcessor for Validator<'_> {
        fn process_variable_expression(&mut self, identifier: &mut Identifier) {
            self.check(identifier);
        }

        fn process_local_assign_statement(&mut self, assignment: &mut VariableAssignment) {
            for variable in assignment.iter_variables() {
                self.check(variable.get_identifier());
            }
        }

        fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
            self.check(function.get_identifier());
            self.check_parameters(function.iter_parameters());
        }

        fn process_function_statement(&mut self, function: &mut FunctionStatement) {
            self.check_parameters(function.iter_parameters());
        }

        fn process_function_expression(&mut self, function: &mut FunctionExpression) {
            self.check_parameters(function.iter_parameters());
        }

        fn process_numeric_for_statement(&mut self, statement: &mut NumericForStatement) {
            self.check(statement.get_identifier().get_identifier());
        }

        fn process_generic_for_statement(&mut self, statement: &mut GenericForStatement) {
            for identifier in statement.iter_identifiers() {
                self.check(identifier.get_identifier());
            }
        }

        fn process_type_function(&mut self, function: &mut TypeFunctionStatement) {
            self.check_parameters(function.iter_parameters());
        }
    }

    let mut validator = Validator { model, error: None };
    DefaultVisitor::visit_block(block, &mut validator);

    match validator.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Renamer<'a> {
    model: &'a SemanticModel,
    renames: &'a HashMap<SymbolId, String>,
}

impl Renamer<'_> {
    fn rename_identifier(&self, identifier: &mut Identifier) {
        let symbol = self
            .model
            .resolve_identifier(identifier)
            .expect("identifiers were validated before the rewrite");
        if let Some(name) = self.renames.get(&symbol) {
            identifier.set_name(name.clone());
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze_with_tokens, generate, parse};
    use crate::semantic::{SemanticAnalyzer, SymbolKind};

    #[test]
    fn renames_locals_consistently() {
        let (mut block, model) = analyze_with_tokens("local long_name = 1\nreturn long_name");

        let renamed =
            rename_variables(&mut block, &model).expect("renaming must succeed with tokens");

        assert_eq!(renamed, 1);
        let output = generate(&block);
        assert!(!output.contains("long_name"), "unexpected: {output}");

        let reparsed = parse(&output);
        let new_model = SemanticAnalyzer::new()
            .analyze(&reparsed)
            .expect("renamed code must analyze");
        let local = new_model
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Local)
            .expect("the renamed local must exist");
        assert_eq!(local.read_count, 1);
        assert_eq!(local.write_count, 1);
    }

    #[test]
    fn preserves_shadowing_bindings() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            local value = 1
            do
                local value = 2
                print(value)
            end
            print(value)
            "#,
        );

        let renamed =
            rename_variables(&mut block, &model).expect("renaming must succeed with tokens");
        assert_eq!(renamed, 2);

        let output = generate(&block);
        let reparsed = parse(&output);
        let new_model = SemanticAnalyzer::new()
            .analyze(&reparsed)
            .expect("renamed code must analyze");

        let mut locals: Vec<_> = new_model
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Local)
            .collect();
        locals.sort_by_key(|symbol| symbol.id);
        assert_eq!(locals.len(), 2);
        assert_ne!(
            locals[0].name, locals[1].name,
            "shadowed locals must receive distinct names"
        );
        assert_eq!(locals[0].read_count, 1);
        assert_eq!(locals[1].read_count, 1);
    }

    #[test]
    fn keeps_global_references() {
        let (mut block, model) = analyze_with_tokens("local print = print\nprint(\"x\")");

        rename_variables(&mut block, &model).expect("renaming must succeed with tokens");

        let output = generate(&block);
        assert_eq!(
            output.matches("print").count(),
            1,
            "only the global reference keeps its name: {output}"
        );
    }

    #[test]
    fn renames_captured_upvalues_consistently() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            local captured = 1
            local function f()
                return captured
            end
            return f
            "#,
        );

        rename_variables(&mut block, &model).expect("renaming must succeed with tokens");

        let output = generate(&block);
        assert!(!output.contains("captured"), "unexpected: {output}");
        assert!(!output.contains("function f"), "unexpected: {output}");

        let reparsed = parse(&output);
        let new_model = SemanticAnalyzer::new()
            .analyze(&reparsed)
            .expect("renamed code must analyze");
        let captured = new_model
            .symbols
            .iter()
            .find(|symbol| symbol.capturing_scopes.len() == 1)
            .expect("the captured symbol must still be captured");
        assert_eq!(captured.read_count, 1);
    }

    #[test]
    fn renames_parameters_and_iteration_variables() {
        let (mut block, model) = analyze_with_tokens(
            r#"
            local function f(long_parameter)
                for index = 1, 10 do
                    return long_parameter + index
                end
            end
            return f
            "#,
        );

        let renamed =
            rename_variables(&mut block, &model).expect("renaming must succeed with tokens");
        assert_eq!(renamed, 3, "function, parameter and iteration variable");

        let output = generate(&block);
        assert!(!output.contains("long_parameter"), "unexpected: {output}");
        assert!(!output.contains("index"), "unexpected: {output}");
        parse(&output);
    }

    #[test]
    fn works_without_token_data() {
        let mut block = parse("local long_name = 1\nreturn long_name");
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .expect("semantic analysis must succeed");

        let renamed =
            rename_variables(&mut block, &model).expect("token-less symbols must be renamed");

        assert!(renamed >= 1);
        assert!(
            !generate(&block).contains("long_name"),
            "the local must be renamed: {}",
            generate(&block)
        );
    }
}
