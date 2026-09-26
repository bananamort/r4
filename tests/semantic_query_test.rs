//! Tests for node-keyed semantic queries and their typed errors.

use r4::nodes::{Block, Identifier, Statement};
use r4::semantic::{ScopeId, ScopeKind, SemanticModel, SemanticQueryError, SymbolId};
use r4::Parser;

fn analyze(code: &str) -> (Block, SemanticModel) {
    Parser::default()
        .parse_analyzed(code)
        .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"))
}

fn first_do_block(block: &Block) -> &Block {
    for statement in block.iter_statements() {
        if let Statement::Do(do_statement) = statement {
            return do_statement.get_block();
        }
    }
    panic!("the test code must contain a do block");
}

#[test]
fn maps_blocks_to_their_scopes() {
    let (block, model) = analyze("do local x = 1 end\nif c then do end end");

    let chunk = model.try_scope_of(&block).expect("the chunk has a scope");
    assert_eq!(chunk, model.chunk_scope());
    assert_eq!(
        model.try_find_scope(&block, r4::semantic::ScopeKind::File),
        Ok(chunk)
    );

    let do_block = first_do_block(&block);
    let do_scope = model
        .try_scope_of(do_block)
        .expect("the do block has a scope");
    assert_ne!(do_scope, chunk);
    assert_eq!(
        model.try_find_scope(do_block, r4::semantic::ScopeKind::Block),
        Ok(do_scope)
    );
}

#[test]
fn missing_scope_kind_is_a_typed_error() {
    let (block, model) = analyze("local x = 1\nreturn x");
    let error = model
        .try_find_scope(&block, r4::semantic::ScopeKind::Block)
        .expect_err("the chunk has no block scope");
    assert!(matches!(error, SemanticQueryError::NoEnclosingScope { .. }));
    assert_eq!(error.code(), "query/no-enclosing-scope");
}

#[test]
fn foreign_models_are_rejected() {
    let (first_block, first_model) = analyze("local x = 1\nreturn x");
    let (second_block, _second_model) = analyze("local x = 1\nreturn x");

    let error = first_model
        .try_scope_of(&second_block)
        .expect_err("a block from another model must be rejected");
    assert!(matches!(error, SemanticQueryError::ForeignBlock { .. }));
    assert_eq!(error.code(), "query/foreign-block");

    // The same block in its own model is fine.
    assert!(first_model.try_scope_of(&first_block).is_ok());
}

#[test]
fn unanalyzed_blocks_and_identifiers_are_rejected() {
    let (block, model) = analyze("local x = 1\nreturn x");

    let fresh = Parser::default()
        .parse("return 1")
        .expect("the fresh block parses");
    assert!(matches!(
        model.try_scope_of(&fresh),
        Err(SemanticQueryError::UnidentifiedBlock)
    ));

    let identifier = Identifier::new("never_bound");
    assert!(matches!(
        model.try_declared_symbol(&identifier),
        Err(SemanticQueryError::UnboundIdentifier { .. })
    ));
    assert!(matches!(
        model.try_binding(&identifier),
        Err(SemanticQueryError::UnboundOccurrence { .. })
    ));

    let _ = block;
}

#[test]
fn resolves_declarations_and_their_scopes() {
    let (block, model) = analyze("local x = 1\nreturn x");

    let statement = block.iter_statements().next().expect("a statement");
    let Statement::LocalAssign(assignment) = statement else {
        panic!("expected a local assignment");
    };
    let identifier = assignment
        .iter_variables()
        .next()
        .expect("a variable")
        .get_identifier();

    let symbol = model
        .try_declared_symbol(identifier)
        .expect("the declaration is bound");
    assert_eq!(model.symbol(symbol).name, "x");
    assert_eq!(
        model.try_scope_of_identifier(identifier),
        Ok(model.symbol(symbol).declaring_scope)
    );
    assert!(matches!(
        model.try_binding(identifier),
        Ok(r4::semantic::Binding::Symbol(_))
    ));
}

#[test]
fn resolves_type_declarations_and_annotations() {
    let (block, model) = analyze("type T = number\nlocal x: T = nil\nreturn x");

    let mut type_name = None;
    let mut annotation = None;
    for statement in block.iter_statements() {
        match statement {
            Statement::TypeDeclaration(declaration) => {
                type_name = Some(declaration.get_name().clone());
            }
            Statement::LocalAssign(assignment) => {
                for variable in assignment.iter_variables() {
                    if let Some(r4::nodes::Type::Name(name)) = variable.get_type() {
                        annotation = Some(name.get_type_name().clone());
                    }
                }
            }
            _ => {}
        }
    }

    let type_name = type_name.expect("a type declaration");
    let type_id = model
        .try_declared_type(&type_name)
        .expect("the declared type resolves");
    assert_eq!(model.type_of(&type_name), Some(type_id));
    assert_eq!(model.resolve_type_expanded(&type_name), Some(type_id));

    let annotation = annotation.expect("a type annotation");
    assert_eq!(model.type_of(&annotation), Some(type_id));
    assert_eq!(model.try_declared_type(&annotation), Ok(type_id));
    assert!(model.try_scope_of_identifier(&type_name).is_ok());

    let _ = model.analysis_origin();

    let unbound = Identifier::new("NeverDeclared");
    assert!(matches!(
        model.try_declared_type(&unbound),
        Err(SemanticQueryError::UnboundType { .. })
    ));
}

#[test]
fn resolves_label_declarations() {
    let block = Parser::default()
        .lua52()
        .parse("::top::\nlocal x = 1\ngoto top")
        .expect("the labeled chunk parses");
    let model = r4::semantic::SemanticAnalyzer::new()
        .analyze(&block)
        .expect("the labeled chunk analyzes");

    let mut label_name = None;
    for statement in block.iter_statements() {
        if let Statement::Label(label) = statement {
            label_name = Some(label.get_label().clone());
        }
    }

    let label = label_name.expect("a label declaration");
    let label_id = model
        .try_declared_label(&label)
        .expect("the label resolves");
    assert_eq!(model.label_of(&label), Some(label_id));
    assert!(model.try_scope_of_identifier(&label).is_ok());

    // Goto occurrences are resolved through the goto binding list.
    let binding = model
        .goto_bindings
        .iter()
        .find(|binding| binding.site.name == "top")
        .expect("the goto is bound");
    assert_eq!(binding.label, label_id);
    assert_eq!(binding.site.span, None);
    assert_eq!(
        model.goto_bindings_in_function(model.chunk_scope()).count(),
        1
    );

    assert!(matches!(
        model.try_declared_label(&Identifier::new("never")),
        Err(SemanticQueryError::UnboundLabel { .. })
    ));
}

#[test]
fn function_scope_of_walks_to_the_enclosing_function() {
    use r4::semantic::ScopeKind;

    let (block, model) =
        analyze("local x = 1\ndo local y = 2 end\nlocal function f() return x end\nreturn f");

    let chunk = model.chunk_scope();
    assert_eq!(model.function_scope_of(chunk), Some(chunk));

    let block_scope = model
        .scopes
        .iter()
        .find(|scope| scope.kind == ScopeKind::Block)
        .map(|scope| scope.id);
    if let Some(block_scope) = block_scope {
        assert_eq!(model.function_scope_of(block_scope), Some(chunk));
    }

    let function_scope = model
        .scopes
        .iter()
        .find(|scope| scope.kind == ScopeKind::Function)
        .map(|scope| scope.id);
    if let Some(function_scope) = function_scope {
        assert_eq!(
            model.function_scope_of(function_scope),
            Some(function_scope)
        );
    }

    let _ = block;
}

fn symbol_named(model: &SemanticModel, scope: ScopeId, name: &str) -> SymbolId {
    model
        .scope(scope)
        .symbols
        .iter()
        .copied()
        .find(|symbol| model.symbol(*symbol).name == name)
        .unwrap_or_else(|| panic!("scope does not declare `{name}`"))
}

fn do_blocks(block: &Block) -> Vec<&Block> {
    block
        .iter_statements()
        .filter_map(|statement| match statement {
            Statement::Do(do_statement) => Some(do_statement.get_block()),
            _ => None,
        })
        .collect()
}

fn scope_of_kind(model: &SemanticModel, kind: ScopeKind) -> ScopeId {
    model
        .scopes
        .iter()
        .find(|scope| scope.kind == kind)
        .unwrap_or_else(|| panic!("the model has no {kind:?} scope"))
        .id
}

#[test]
fn scope_kind_rank_orders_global_to_block() {
    assert!(ScopeKind::Global.rank() < ScopeKind::File.rank());
    assert!(ScopeKind::File.rank() < ScopeKind::Function.rank());
    assert!(ScopeKind::Function.rank() < ScopeKind::Block.rank());
}

#[test]
fn find_variable_is_bounded_by_scope_kind() {
    let (_, model) = analyze("local a = 1\nlocal function f(b)\n\tdo\n\t\tlocal c = 3\n\tend\nend");
    let block = scope_of_kind(&model, ScopeKind::Block);

    // The starting scope is always searched, regardless of `kind`.
    assert!(model
        .try_find_variable(block, "c", ScopeKind::Block)
        .expect("the scope belongs to the model")
        .is_some());
    assert!(model
        .try_find_variable(block, "c", ScopeKind::Global)
        .expect("the scope belongs to the model")
        .is_some());

    // The parameter is reachable at `Function` but not through a `Block` bound.
    assert!(model
        .try_find_variable(block, "b", ScopeKind::Function)
        .expect("the scope belongs to the model")
        .is_some());
    assert!(model
        .try_find_variable(block, "b", ScopeKind::Block)
        .expect("the scope belongs to the model")
        .is_none());

    // The file local is reachable at `File` but never through a `Function` bound.
    assert!(model
        .try_find_variable(block, "a", ScopeKind::File)
        .expect("the scope belongs to the model")
        .is_some());
    assert!(model
        .try_find_variable(block, "a", ScopeKind::Function)
        .expect("the scope belongs to the model")
        .is_none());

    assert!(model
        .try_find_variable(block, "missing", ScopeKind::Global)
        .expect("the scope belongs to the model")
        .is_none());
}

#[test]
fn find_variable_returns_the_most_recent_declaration() {
    let (_, model) = analyze("local x = 1\nlocal x = 2\nreturn x");
    let file = model.chunk_scope();
    let symbols = &model.scope(file).symbols;
    assert_eq!(symbols.len(), 2);

    let found = model
        .try_find_variable(file, "x", ScopeKind::Block)
        .expect("the scope belongs to the model")
        .expect("x is declared");
    assert_eq!(
        found, symbols[1],
        "the latest declaration shadows the first"
    );
}

#[test]
fn accessibility_descends_and_stops_at_siblings() {
    let (block, model) = analyze("local a = 1\ndo local b = 2 end\ndo local c = 3 end");
    let file = model.chunk_scope();
    let blocks = do_blocks(&block);
    assert_eq!(blocks.len(), 2);
    let first = model
        .try_scope_of(blocks[0])
        .expect("the do block has a scope");
    let second = model
        .try_scope_of(blocks[1])
        .expect("the do block has a scope");

    let a = symbol_named(&model, file, "a");
    let b = symbol_named(&model, first, "b");
    let c = symbol_named(&model, second, "c");

    assert!(model
        .try_can_be_accessed_in(a, file)
        .expect("the scope belongs to the model"));
    assert!(model
        .try_can_be_accessed_in(a, first)
        .expect("the scope belongs to the model"));
    assert!(model
        .try_can_be_accessed_in(a, second)
        .expect("the scope belongs to the model"));

    assert!(model
        .try_can_be_accessed_in(b, first)
        .expect("the scope belongs to the model"));
    assert!(!model
        .try_can_be_accessed_in(b, file)
        .expect("the scope belongs to the model"));
    assert!(!model
        .try_can_be_accessed_in(b, second)
        .expect("the scope belongs to the model"));

    assert!(model
        .try_can_be_accessed_in(c, second)
        .expect("the scope belongs to the model"));
    assert!(!model
        .try_can_be_accessed_in(c, first)
        .expect("the scope belongs to the model"));
}

#[test]
fn accessibility_stops_at_function_boundaries() {
    let (_, model) = analyze("local a = 1\nlocal function f() local b = 2 return a + b end");
    let file = model.chunk_scope();
    let function = scope_of_kind(&model, ScopeKind::Function);

    let a = symbol_named(&model, file, "a");
    let b = symbol_named(&model, function, "b");

    // A file local is visible inside the nested function.
    assert!(model
        .try_can_be_accessed_in(a, function)
        .expect("the scope belongs to the model"));
    // A function local is not visible outside of it.
    assert!(model
        .try_can_be_accessed_in(b, function)
        .expect("the scope belongs to the model"));
    assert!(!model
        .try_can_be_accessed_in(b, file)
        .expect("the scope belongs to the model"));
}

#[test]
fn scoping_queries_fail_fast_on_unknown_ids() {
    let (_, model) = analyze("local x = 1");

    let error = model
        .try_find_variable(ScopeId(9_999), "x", ScopeKind::Block)
        .expect_err("an unknown scope is rejected");
    assert_eq!(error.code(), "query/unknown-scope");
    assert!(matches!(error, SemanticQueryError::UnknownScope { .. }));

    let error = model
        .try_can_be_accessed_in(SymbolId(9_999), model.chunk_scope())
        .expect_err("an unknown symbol is rejected");
    assert_eq!(error.code(), "query/unknown-symbol");
    assert!(matches!(error, SemanticQueryError::UnknownSymbol { .. }));

    let declared = symbol_named(&model, model.chunk_scope(), "x");
    let error = model
        .try_can_be_accessed_in(declared, ScopeId(9_999))
        .expect_err("an unknown scope is rejected");
    assert_eq!(error.code(), "query/unknown-scope");
}

#[test]
fn arena_accessors_have_fallible_variants() {
    use r4::semantic::{LabelId, TypeId};

    let (_, model) = analyze("local x = 1\nreturn x");

    // Identifiers from another model must be answerable without a panic.
    let missing = SymbolId(9_999);
    assert!(model.try_symbol(missing).is_none());
    assert!(model.try_scope(ScopeId(9_999)).is_none());
    assert!(model.try_label(LabelId(9_999)).is_none());
    assert!(model.try_type_symbol(TypeId(9_999)).is_none());
    assert!(model.try_uses_of(missing).is_none());
    assert!(model.try_is_unused(missing).is_none());
    assert!(model.try_is_immutable(missing).is_none());
    assert!(model.try_immutable_alias_root(missing).is_none());
    assert!(model.try_immutable_global_alias(missing).is_none());

    // Real identifiers still resolve, and the predicates agree with the symbol.
    let x = model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "x")
        .expect("x")
        .id;
    assert_eq!(model.try_symbol(x).map(|symbol| symbol.id), Some(x));
    assert_eq!(model.try_is_unused(x), Some(model.is_unused(x)));
    assert_eq!(model.try_is_immutable(x), Some(model.is_immutable(x)));
}
