//! API coverage for the semantic model, the unified module hierarchy and
//! escape analysis over field/index writes.

use r4::nodes::Identifier;
use r4::process::{ImmutableProcessor, ImmutableVisitor, Walk};
use r4::semantic::{
    Binding, EscapeAnalysis, ModuleError, MultiModuleModel, SemanticAnalyzer, SemanticModel,
    SymbolRef,
};
use r4::{Dialect, Parser};

struct Prober<'a> {
    model: &'a SemanticModel,
    saw_symbol: bool,
    saw_label: bool,
    saw_type: bool,
}

impl ImmutableProcessor for Prober<'_> {
    fn process_identifier(&mut self, identifier: &Identifier) {
        if let Some(binding) = self.model.binding_of(identifier) {
            match binding {
                Binding::Symbol(_) => self.saw_symbol = true,
                Binding::Label(_) => self.saw_label = true,
                Binding::Type(_) => self.saw_type = true,
            }
            let _ = binding.as_symbol();
            let _ = binding.as_label();
            let _ = binding.as_type();
        }
        let _ = self.model.symbol_of(identifier);
        let _ = self.model.label_of(identifier);
        let _ = self.model.resolve_label(identifier);
        let _ = self.model.resolve_type(identifier);
        let _ = self.model.resolve_type_expanded(identifier);
    }
}

#[test]
fn binding_and_resolution_accessors() {
    // Value and type bindings are Luau; labels are Lua 5.2.
    for (code, dialect) in [
        ("local x = 1\ntype T = number\nreturn x", Dialect::Luau),
        ("::done::\ngoto done\nreturn 1", Dialect::Lua52),
    ] {
        let block = Parser::default()
            .dialect(dialect)
            .preserve_tokens()
            .parse(code)
            .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));

        let mut prober = Prober {
            model: &model,
            saw_symbol: false,
            saw_label: false,
            saw_type: false,
        };
        Walk::visit_block(&block, &mut prober);

        match dialect {
            Dialect::Luau => {
                assert!(prober.saw_symbol, "a value binding must be seen");
                assert!(prober.saw_type, "a type binding must be seen");
            }
            Dialect::Lua52 => assert!(prober.saw_label, "a label binding must be seen"),
            _ => {}
        }
    }
}

#[test]
fn unified_hierarchy_accessors_and_module_errors() {
    let mut graph = MultiModuleModel::new();
    let (block, model) = Parser::default()
        .parse_analyzed("local v = 1\nprint(v)\nreturn v")
        .expect("analysis succeeds");
    let module = graph
        .add_module("main", block, model)
        .expect("the module is valid");

    let global = graph.global_symbol("print").expect("print is global");
    let global_ref = SymbolRef::Global(global);
    assert!(global_ref.is_global());
    assert!(global_ref.as_local().is_none());

    let local_symbol = graph
        .module(module)
        .model
        .symbols
        .iter()
        .find(|symbol| symbol.name == "v")
        .expect("the local must exist")
        .id;
    let local_ref = SymbolRef::Local {
        module,
        symbol: local_symbol,
    };
    assert!(!local_ref.is_global());
    assert_eq!(local_ref.as_local(), Some((module, local_symbol)));

    for value in graph.module(module).interface.iter_values() {
        let _ = value;
    }

    let errors = [
        ModuleError::UnboundExport {
            name: "x".to_owned(),
        },
        ModuleError::UnknownExport {
            module: "m".to_owned(),
            name: "y".to_owned(),
        },
        ModuleError::UnknownImportAlias {
            module: "m".to_owned(),
            name: "z".to_owned(),
        },
        ModuleError::UnknownImportedType {
            module: "m".to_owned(),
            name: "T".to_owned(),
        },
        ModuleError::AmbiguousImportedType {
            name: "T".to_owned(),
            modules: vec!["a".to_owned(), "b".to_owned()],
        },
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn escape_analysis_records_field_and_index_writes() {
    for code in [
        "local t = {}\nt.x = 1\nreturn 0",
        "local t = {}\nt[1] = 2\nreturn 0",
        "local t = {}\nt[k] = 3\nreturn 0",
        "local t = {}\nt.a = 1\nt[1] = 2\nt[k] = 3\nreturn 0",
    ] {
        let (block, model) = Parser::default()
            .parse_analyzed(code)
            .unwrap_or_else(|error| panic!("failed to analyze `{code}`: {error}"));
        let analysis = EscapeAnalysis::analyze(&block, &model);
        let t = model
            .symbols
            .iter()
            .find(|symbol| symbol.name == "t")
            .expect("the local must exist")
            .id;
        assert!(analysis.is_table(t), "`{code}` must be a table");
        assert!(
            analysis.is_confined_table(&model, t),
            "`{code}` must stay confined"
        );
    }

    // `Dialect` is referenced to keep the import meaningful across the corpus.
    let _ = Dialect::Lua52;
}
