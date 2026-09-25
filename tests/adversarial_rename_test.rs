//! Adversarial end-to-end test for variable renaming.
//!
//! The fixture (`fixtures/adversarial_rename.lua`) is a dense program whose
//! identifiers are tagged by the convention the file documents:
//!
//! * `SAFE_<N>` marks renamable symbols — locals, parameters, loop variables and
//!   upvalues (including the `self` a colon method receives implicitly);
//! * `UNSAFE_<N>` marks names that must survive — globals, table keys, method
//!   names, metamethods and external API fields.
//!
//! The test renames the program, re-parses and re-analyzes the output, and
//! checks the semantic structure rather than the generated text: every renamable
//! symbol is renamed, no `SAFE_` symbol survives, the implicit receivers keep
//! `self`, globals are unchanged, and every field/key/metamethod name is
//! byte-identical.

use std::collections::BTreeSet;

use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::nodes::{Block, Expression, FunctionCall, Prefix, Statement, TableEntry, TableExpression};
use r4::process::{ImmutableProcessor, ImmutableVisitor, Walk};
use r4::rules::rename_variables::rename_variables;
use r4::semantic::{AccessKind, SemanticAnalyzer, SemanticModel, SymbolKind};
use r4::Parser;

const FIXTURE: &str = include_str!("fixtures/adversarial_rename.lua");

fn parse(code: &str) -> Block {
    Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse: {error}"))
}

fn analyze(block: &Block) -> SemanticModel {
    SemanticAnalyzer::new()
        .analyze(block)
        .unwrap_or_else(|error| panic!("semantic analysis failed: {error}"))
}

fn generate(block: &Block) -> String {
    let mut generator = ReadableLuaGenerator::default();
    generator.write_block(block);
    generator.into_string()
}

/// Every name that appears in a non-variable position: table field keys, string
/// index keys, method names, function field names and field-expression accesses.
/// None of these is a variable, so renaming must not touch any of them.
#[derive(Default)]
struct NonVariableNames {
    names: BTreeSet<String>,
}

impl ImmutableProcessor for NonVariableNames {
    fn process_statement(&mut self, statement: &Statement) {
        let Statement::Function(function) = statement else {
            return;
        };
        let name = function.get_name();
        self.names
            .extend(name.get_field_names().iter().map(|i| i.get_name().clone()));
        if let Some(method) = name.get_method() {
            self.names.insert(method.get_name().clone());
        }
    }

    fn process_expression(&mut self, expression: &Expression) {
        if let Expression::Field(field) = expression {
            self.names.insert(field.get_field().get_name().clone());
        }
    }

    fn process_prefix(&mut self, prefix: &Prefix) {
        if let Prefix::Field(field) = prefix {
            self.names.insert(field.get_field().get_name().clone());
        }
    }

    fn process_function_call(&mut self, call: &FunctionCall) {
        if let Some(method) = call.get_method() {
            self.names.insert(method.get_name().clone());
        }
    }

    fn process_table(&mut self, table: &TableExpression) {
        for entry in table.iter_entries() {
            match entry {
                TableEntry::Field(field) => {
                    self.names.insert(field.get_field().get_name().clone());
                }
                TableEntry::Index(index) => {
                    if let Expression::String(string) = index.get_key() {
                        if let Some(value) = string.get_string_value() {
                            self.names.insert(value.to_owned());
                        }
                    }
                }
                TableEntry::Value(_) => {}
            }
        }
    }
}

fn non_variable_names(block: &Block) -> BTreeSet<String> {
    let mut collector = NonVariableNames::default();
    Walk::visit_block(block, &mut collector);
    collector.names
}

fn global_names(model: &SemanticModel) -> BTreeSet<String> {
    model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Global)
        .map(|symbol| symbol.name.clone())
        .collect()
}

fn access_totals(model: &SemanticModel) -> (usize, usize, usize) {
    let mut declarations = 0;
    let mut writes = 0;
    let mut reads = 0;
    for symbol in &model.symbols {
        for usage in model.uses_of(symbol.id) {
            match usage.kind {
                AccessKind::Declaration => declarations += 1,
                AccessKind::Write => writes += 1,
                AccessKind::Read => reads += 1,
            }
        }
    }
    (declarations, writes, reads)
}

fn receivers(model: &SemanticModel) -> Vec<usize> {
    let mut counts: Vec<usize> = model
        .symbols
        .iter()
        .filter(|symbol| symbol.is_implicit())
        .map(|symbol| symbol.read_count)
        .collect();
    counts.sort_unstable();
    counts
}

#[test]
fn adversarial_program_survives_renaming() {
    let block = parse(FIXTURE);
    let model = analyze(&block);

    // The fixture must actually exercise the colon-method receiver.
    assert!(
        model.symbols.iter().any(|symbol| symbol.is_implicit()),
        "the fixture must contain at least one implicit receiver"
    );

    let renamable = model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind != SymbolKind::Global && !symbol.is_implicit())
        .count();

    let before_keys = non_variable_names(&block);
    let before_globals = global_names(&model);
    let before_receivers = receivers(&model);
    let before_access = access_totals(&model);

    let mut renamed_block = block.clone();
    let renamed = rename_variables(&mut renamed_block, &model).expect("renaming must succeed");
    assert_eq!(
        renamed, renamable,
        "every renamable symbol must be renamed exactly once"
    );

    let output = generate(&renamed_block);

    // The output must re-parse and re-analyze cleanly.
    let output_block = parse(&output);
    let output_model = analyze(&output_block);

    // No `SAFE_` symbol survives, and no `UNSAFE_` local was preserved by name.
    for symbol in &output_model.symbols {
        assert!(
            !symbol.name.starts_with("SAFE_"),
            "`{}` should have been renamed",
            symbol.name
        );
    }
    // Field keys, method names, metamethods and string keys are byte-identical.
    assert_eq!(
        non_variable_names(&output_block),
        before_keys,
        "renaming must not touch field/key/metamethod names"
    );
    for preserved in [
        "__index",
        "__newindex",
        "__call",
        "__tostring",
        "__concat",
        "__add",
    ] {
        assert!(
            before_keys.contains(preserved),
            "the fixture must contain the `{preserved}` metamethod"
        );
        assert!(
            non_variable_names(&output_block).contains(preserved),
            "the metamethod `{preserved}` must survive"
        );
    }

    // Globals keep their names.
    assert_eq!(
        global_names(&output_model),
        before_globals,
        "globals must keep their names"
    );
    assert!(before_globals.contains("UNSAFE_1"));
    assert!(before_globals.contains("UNSAFE_global_state"));

    // The implicit colon receivers stay `self` with the same use counts.
    assert_eq!(
        receivers(&output_model),
        before_receivers,
        "implicit receivers must keep `self` and their uses"
    );
    assert!(output_model
        .symbols
        .iter()
        .any(|symbol| { symbol.is_implicit() && symbol.name == "self" && symbol.read_count > 0 }));

    // Def-use structure and symbol count are unchanged by renaming.
    assert_eq!(
        access_totals(&output_model),
        before_access,
        "renaming must not change the def-use structure"
    );
    assert_eq!(
        output_model.symbols.len(),
        model.symbols.len(),
        "renaming must not add or remove symbols"
    );

    // The three `UNSAFE_`-named locals are locals despite the fixture's label, so
    // a symbol-based rename renames them (which is always safe). This documents
    // the fixture's convention rather than applying a name-based exemption.
    let unsafe_locals: BTreeSet<&str> = model
        .symbols
        .iter()
        .filter(|symbol| symbol.kind != SymbolKind::Global && symbol.name.starts_with("UNSAFE_"))
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(
        unsafe_locals,
        BTreeSet::from(["UNSAFE_game", "UNSAFE_Instance", "UNSAFE_TweenInfo"]),
        "only these `UNSAFE_` names are locals; the rest are globals or field keys"
    );
    for local in unsafe_locals {
        assert!(
            !output_model
                .symbols
                .iter()
                .any(|symbol| symbol.name == local),
            "the local `{local}` is renamed like any other local"
        );
    }
}
