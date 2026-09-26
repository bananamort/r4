//! Enforced construct coverage.
//!
//! Rust has no reflection, so coverage is enforced by construction: every
//! registry below maps an AST enum to a tag enum with an **exhaustive** `of`
//! match (no wildcard), so adding an AST variant is a compile error until it is
//! registered, and the tag list is generated from the same invocation so it
//! cannot drift from the match.
//!
//! `every_variant_is_reached_by_the_corpus` then asserts that the corpus
//! exercises every registered tag. Adding a variant therefore fails the build
//! until (a) it is registered and (b) an input reaches it.
//!
//! `the_corpus_flows_through_the_whole_pipeline` runs every corpus program
//! through parse → analyze (arena/scope invariants) → CFG (invariants) →
//! liveness/constant-propagation → generation → re-parse equivalence, so a
//! registered construct is also exercised end to end instead of merely named.

use std::collections::BTreeSet;

use r4::cfg::analysis::{ConstantPropagation, Liveness};
use r4::cfg::build_chunk;
use r4::nodes::{
    Arguments, AssignmentKind, Attribute, AttributeArguments, AttributeGroup,
    AttributeGroupElement, AttributeTupleArguments, Attributes, BinaryExpression, BinaryOperator,
    Block, ChildVisitor, CompoundOperator, DecimalNumber, Expression, FunctionReturnType,
    FunctionStatement, FunctionVariadicType, GenericParameter, GenericParameterRef,
    GenericParameters, GenericParametersWithDefaults, GenericTypePack, GenericTypePackDefault,
    Identifier, InterpolationSegment, LastStatement, LiteralExpression, LiteralTable,
    LiteralTableEntry, NumberExpression, Prefix, ReadOnlyChildren, ReturnStatement, Statement,
    StringExpression, TableEntry, TableEntryType, TableExpression, TablePropertyModifier,
    TriviaKind, Type, TypeName, TypePack, TypeParameter, TypedIdentifier, UnaryExpression,
    UnaryOperator, Variable, VariadicArgumentType, VariadicTypePack,
};
use r4::process::collect_trivia;
use r4::semantic::SemanticAnalyzer;
use r4::{Equivalent, Parser};

/// Declares a tag registry for an AST enum.
///
/// `of` is an exhaustive match (no `_`), `ALL` is generated from the same arm
/// list, and `name` is exhaustive too, so the registry cannot silently miss a
/// variant.
macro_rules! variant_registry {
    ($tag:ident, $ast:ty, $( $variant:ident => $pattern:pat ),* $(,)?) => {
        #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
        #[allow(clippy::enum_variant_names)]
        enum $tag { $( $variant ),* }

        impl $tag {
            const ALL: &'static [$tag] = &[ $( $tag::$variant ),* ];

            fn name(self) -> &'static str {
                match self { $( $tag::$variant => stringify!($variant) ),* }
            }

            fn of(value: &$ast) -> Self {
                match value { $( $pattern => $tag::$variant ),* }
            }
        }
    };
}

variant_registry!(StatementTag, Statement,
    Assign => Statement::Assign(_),
    Do => Statement::Do(_),
    Call => Statement::Call(_),
    CompoundAssign => Statement::CompoundAssign(_),
    Function => Statement::Function(_),
    Goto => Statement::Goto(_),
    GenericFor => Statement::GenericFor(_),
    If => Statement::If(_),
    Label => Statement::Label(_),
    LocalAssign => Statement::LocalAssign(_),
    LocalFunction => Statement::LocalFunction(_),
    NumericFor => Statement::NumericFor(_),
    Repeat => Statement::Repeat(_),
    While => Statement::While(_),
    TypeDeclaration => Statement::TypeDeclaration(_),
    TypeFunction => Statement::TypeFunction(_),
);

variant_registry!(ExpressionTag, Expression,
    Binary => Expression::Binary(_),
    Call => Expression::Call(_),
    False => Expression::False(_),
    Field => Expression::Field(_),
    Function => Expression::Function(_),
    Identifier => Expression::Identifier(_),
    If => Expression::If(_),
    Index => Expression::Index(_),
    Nil => Expression::Nil(_),
    Number => Expression::Number(_),
    Parenthese => Expression::Parenthese(_),
    String => Expression::String(_),
    InterpolatedString => Expression::InterpolatedString(_),
    Table => Expression::Table(_),
    True => Expression::True(_),
    Unary => Expression::Unary(_),
    VariableArguments => Expression::VariableArguments(_),
    TypeCast => Expression::TypeCast(_),
    TypeInstantiation => Expression::TypeInstantiation(_),
);

variant_registry!(PrefixTag, Prefix,
    Call => Prefix::Call(_),
    Field => Prefix::Field(_),
    Identifier => Prefix::Identifier(_),
    Index => Prefix::Index(_),
    Parenthese => Prefix::Parenthese(_),
    TypeInstantiation => Prefix::TypeInstantiation(_),
);

variant_registry!(VariableTag, Variable,
    Identifier => Variable::Identifier(_),
    Field => Variable::Field(_),
    Index => Variable::Index(_),
);

variant_registry!(TypeTag, Type,
    Name => Type::Name(_),
    Field => Type::Field(_),
    True => Type::True(_),
    False => Type::False(_),
    Nil => Type::Nil(_),
    String => Type::String(_),
    Array => Type::Array(_),
    Table => Type::Table(_),
    TypeOf => Type::TypeOf(_),
    Parenthese => Type::Parenthese(_),
    Function => Type::Function(_),
    Optional => Type::Optional(_),
    Intersection => Type::Intersection(_),
    Union => Type::Union(_),
);

variant_registry!(LastStatementTag, LastStatement,
    Break => LastStatement::Break(_),
    Continue => LastStatement::Continue(_),
    Return => LastStatement::Return(_),
);

variant_registry!(LiteralExpressionTag, LiteralExpression,
    True => LiteralExpression::True(_),
    False => LiteralExpression::False(_),
    Nil => LiteralExpression::Nil(_),
    Number => LiteralExpression::Number(_),
    String => LiteralExpression::String(_),
    Table => LiteralExpression::Table(_),
);

variant_registry!(TableEntryTag, TableEntry,
    Field => TableEntry::Field(_),
    Index => TableEntry::Index(_),
    Value => TableEntry::Value(_),
);

variant_registry!(LiteralTableEntryTag, LiteralTableEntry,
    Field => LiteralTableEntry::Field(_),
    Value => LiteralTableEntry::Value(_),
);

variant_registry!(TableEntryTypeTag, TableEntryType,
    Property => TableEntryType::Property(_),
    Literal => TableEntryType::Literal(_),
    Indexer => TableEntryType::Indexer(_),
);

variant_registry!(TablePropertyModifierTag, TablePropertyModifier,
    Read => TablePropertyModifier::Read,
    Write => TablePropertyModifier::Write,
);

variant_registry!(InterpolationSegmentTag, InterpolationSegment,
    String => InterpolationSegment::String(_),
    Value => InterpolationSegment::Value(_),
);

variant_registry!(AttributeTag, Attribute,
    Name => Attribute::Name(_),
    Group => Attribute::Group(_),
);

variant_registry!(AttributeArgumentsTag, AttributeArguments,
    Tuple => AttributeArguments::Tuple(_),
    String => AttributeArguments::String(_),
    Table => AttributeArguments::Table(_),
);

variant_registry!(NumberExpressionTag, NumberExpression,
    Decimal => NumberExpression::Decimal(_),
    Hex => NumberExpression::Hex(_),
    Binary => NumberExpression::Binary(_),
);

variant_registry!(BinaryOperatorTag, BinaryOperator,
    And => BinaryOperator::And,
    Or => BinaryOperator::Or,
    Equal => BinaryOperator::Equal,
    NotEqual => BinaryOperator::NotEqual,
    LowerThan => BinaryOperator::LowerThan,
    LowerOrEqualThan => BinaryOperator::LowerOrEqualThan,
    GreaterThan => BinaryOperator::GreaterThan,
    GreaterOrEqualThan => BinaryOperator::GreaterOrEqualThan,
    Plus => BinaryOperator::Plus,
    Minus => BinaryOperator::Minus,
    Asterisk => BinaryOperator::Asterisk,
    Slash => BinaryOperator::Slash,
    DoubleSlash => BinaryOperator::DoubleSlash,
    Percent => BinaryOperator::Percent,
    Caret => BinaryOperator::Caret,
    Concat => BinaryOperator::Concat,
    BitwiseOr => BinaryOperator::BitwiseOr,
    BitwiseXor => BinaryOperator::BitwiseXor,
    BitwiseAnd => BinaryOperator::BitwiseAnd,
    ShiftLeft => BinaryOperator::ShiftLeft,
    ShiftRight => BinaryOperator::ShiftRight,
);

variant_registry!(UnaryOperatorTag, UnaryOperator,
    Length => UnaryOperator::Length,
    Minus => UnaryOperator::Minus,
    Not => UnaryOperator::Not,
    BitwiseNot => UnaryOperator::BitwiseNot,
);

variant_registry!(CompoundOperatorTag, CompoundOperator,
    Plus => CompoundOperator::Plus,
    Minus => CompoundOperator::Minus,
    Asterisk => CompoundOperator::Asterisk,
    Slash => CompoundOperator::Slash,
    DoubleSlash => CompoundOperator::DoubleSlash,
    Percent => CompoundOperator::Percent,
    Caret => CompoundOperator::Caret,
    Concat => CompoundOperator::Concat,
);

variant_registry!(AssignmentKindTag, AssignmentKind,
    Local => AssignmentKind::Local,
    Const => AssignmentKind::Const,
);

variant_registry!(FunctionReturnTypeTag, FunctionReturnType,
    Type => FunctionReturnType::Type(_),
    TypePack => FunctionReturnType::TypePack(_),
    GenericTypePack => FunctionReturnType::GenericTypePack(_),
    VariadicTypePack => FunctionReturnType::VariadicTypePack(_),
);

variant_registry!(VariadicArgumentTypeTag, VariadicArgumentType,
    GenericTypePack => VariadicArgumentType::GenericTypePack(_),
    VariadicTypePack => VariadicArgumentType::VariadicTypePack(_),
);

variant_registry!(FunctionVariadicTypeTag, FunctionVariadicType,
    Type => FunctionVariadicType::Type(_),
    GenericTypePack => FunctionVariadicType::GenericTypePack(_),
);

variant_registry!(TypeParameterTag, TypeParameter,
    Type => TypeParameter::Type(_),
    TypePack => TypeParameter::TypePack(_),
    VariadicTypePack => TypeParameter::VariadicTypePack(_),
    GenericTypePack => TypeParameter::GenericTypePack(_),
);

variant_registry!(GenericParameterTag, GenericParameterRef<'_>,
    TypeVariable => GenericParameterRef::TypeVariable(_),
    TypeVariableWithDefault => GenericParameterRef::TypeVariableWithDefault(_),
    GenericTypePack => GenericParameterRef::GenericTypePack(_),
    GenericTypePackWithDefault => GenericParameterRef::GenericTypePackWithDefault(_),
);

variant_registry!(ArgumentsTag, Arguments,
    Tuple => Arguments::Tuple(_),
    String => Arguments::String(_),
    Table => Arguments::Table(_),
);

variant_registry!(GenericTypePackDefaultTag, GenericTypePackDefault,
    TypePack => GenericTypePackDefault::TypePack(_),
    VariadicTypePack => GenericTypePackDefault::VariadicTypePack(_),
    GenericTypePack => GenericTypePackDefault::GenericTypePack(_),
);

variant_registry!(OwnedGenericParameterTag, GenericParameter,
    TypeVariable => GenericParameter::TypeVariable(_),
    TypeVariableWithDefault => GenericParameter::TypeVariableWithDefault(_),
    GenericTypePack => GenericParameter::GenericTypePack(_),
    GenericTypePackWithDefault => GenericParameter::GenericTypePackWithDefault(_),
);

variant_registry!(TriviaKindTag, TriviaKind,
    Shebang => TriviaKind::Shebang,
    Comment => TriviaKind::Comment,
    Whitespace => TriviaKind::Whitespace,
    EndOfLine => TriviaKind::EndOfLine,
);

/// Every registry check name, so the coverage report can require all of them.
fn all_tags() -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    macro_rules! push {
        ($($tag:ident),* $(,)?) => { $( for value in $tag::ALL { tags.insert(format!("{}::{}", stringify!($tag), value.name())); } )* };
    }
    push!(
        StatementTag,
        ExpressionTag,
        PrefixTag,
        VariableTag,
        TypeTag,
        LastStatementTag,
        LiteralExpressionTag,
        TableEntryTag,
        LiteralTableEntryTag,
        TableEntryTypeTag,
        TablePropertyModifierTag,
        InterpolationSegmentTag,
        AttributeTag,
        AttributeArgumentsTag,
        NumberExpressionTag,
        BinaryOperatorTag,
        UnaryOperatorTag,
        CompoundOperatorTag,
        AssignmentKindTag,
        FunctionReturnTypeTag,
        VariadicArgumentTypeTag,
        FunctionVariadicTypeTag,
        TypeParameterTag,
        GenericParameterTag,
        OwnedGenericParameterTag,
        ArgumentsTag,
        GenericTypePackDefaultTag,
        TriviaKindTag,
    );
    tags
}

macro_rules! record {
    ($set:expr, $tag:ident, $value:expr) => {
        $set.insert(format!("{}::{}", stringify!($tag), $tag::of($value).name()));
    };
}

/// Recurses through the authoritative child definition and records the tag of
/// every construct it meets, including the enums that are not themselves child
/// callbacks (operators, entry kinds, modifiers, segments).
#[derive(Default)]
struct TagCollector {
    seen: BTreeSet<String>,
}

impl ChildVisitor for TagCollector {
    fn statement(&mut self, statement: &Statement) {
        record!(self.seen, StatementTag, statement);
        match statement {
            Statement::LocalAssign(assign) => {
                record!(self.seen, AssignmentKindTag, &assign.get_assignment_kind());
            }
            Statement::LocalFunction(function) => {
                record!(
                    self.seen,
                    AssignmentKindTag,
                    &function.get_assignment_kind()
                );
            }
            Statement::CompoundAssign(assign) => {
                record!(self.seen, CompoundOperatorTag, &assign.get_operator());
            }
            _ => {}
        }
        statement.for_each_child(self);
    }

    fn last_statement(&mut self, statement: &LastStatement) {
        record!(self.seen, LastStatementTag, statement);
        statement.for_each_child(self);
    }

    fn expression(&mut self, expression: &Expression) {
        record!(self.seen, ExpressionTag, expression);
        match expression {
            Expression::Binary(binary) => {
                record!(self.seen, BinaryOperatorTag, &binary.operator());
            }
            Expression::Unary(unary) => {
                record!(self.seen, UnaryOperatorTag, &unary.operator());
            }
            Expression::Number(number) => {
                record!(self.seen, NumberExpressionTag, number);
            }
            Expression::InterpolatedString(string) => {
                for segment in string.iter_segments() {
                    record!(self.seen, InterpolationSegmentTag, segment);
                }
            }
            Expression::Table(table) => {
                for entry in table.iter_entries() {
                    record!(self.seen, TableEntryTag, entry);
                }
            }
            _ => {}
        }
        expression.for_each_child(self);
    }

    fn prefix(&mut self, prefix: &Prefix) {
        record!(self.seen, PrefixTag, prefix);
        prefix.for_each_child(self);
    }

    fn variable(&mut self, variable: &Variable) {
        record!(self.seen, VariableTag, variable);
        variable.for_each_child(self);
    }

    fn r#type(&mut self, r#type: &Type) {
        record!(self.seen, TypeTag, r#type);
        if let Type::Table(table) = r#type {
            for entry in table.iter_entries() {
                record!(self.seen, TableEntryTypeTag, entry);
                let modifier = match entry {
                    TableEntryType::Property(property) => property.get_modifier(),
                    TableEntryType::Literal(property) => property.get_modifier(),
                    TableEntryType::Indexer(indexer) => indexer.get_modifier(),
                };
                if let Some(modifier) = modifier {
                    record!(self.seen, TablePropertyModifierTag, modifier);
                }
            }
        }
        r#type.for_each_child(self);
    }

    fn type_name(&mut self, type_name: &TypeName) {
        if let Some(parameters) = type_name.get_type_parameters() {
            for parameter in parameters.iter() {
                record!(self.seen, TypeParameterTag, parameter);
            }
        }
        type_name.for_each_child(self);
    }

    fn function_return_type(&mut self, return_type: &FunctionReturnType) {
        record!(self.seen, FunctionReturnTypeTag, return_type);
        return_type.for_each_child(self);
    }

    fn variadic_argument_type(&mut self, argument: &VariadicArgumentType) {
        record!(self.seen, VariadicArgumentTypeTag, argument);
        argument.for_each_child(self);
    }

    fn function_variadic_type(&mut self, variadic: &FunctionVariadicType) {
        record!(self.seen, FunctionVariadicTypeTag, variadic);
        variadic.for_each_child(self);
    }

    fn generic_parameters(&mut self, parameters: &GenericParameters) {
        for identifier in parameters.iter_type_variable() {
            self.seen.insert(format!(
                "GenericParameterTag::{}",
                GenericParameterTag::TypeVariable.name()
            ));
            record!(
                self.seen,
                OwnedGenericParameterTag,
                &GenericParameter::TypeVariable(identifier.clone())
            );
        }
        for generic in parameters.iter_generic_type_pack() {
            self.seen.insert(format!(
                "GenericParameterTag::{}",
                GenericParameterTag::GenericTypePack.name()
            ));
            record!(
                self.seen,
                OwnedGenericParameterTag,
                &GenericParameter::GenericTypePack(Box::new(generic.clone()))
            );
        }
        parameters.for_each_child(self);
    }

    fn generic_parameters_with_defaults(&mut self, parameters: &GenericParametersWithDefaults) {
        for parameter in parameters.iter() {
            record!(self.seen, GenericParameterTag, &parameter);
            let owned = match &parameter {
                GenericParameterRef::TypeVariable(identifier) => {
                    GenericParameter::TypeVariable((*identifier).clone())
                }
                GenericParameterRef::TypeVariableWithDefault(default) => {
                    GenericParameter::TypeVariableWithDefault(Box::new((*default).clone()))
                }
                GenericParameterRef::GenericTypePack(generic) => {
                    GenericParameter::GenericTypePack(Box::new((*generic).clone()))
                }
                GenericParameterRef::GenericTypePackWithDefault(default) => {
                    record!(
                        self.seen,
                        GenericTypePackDefaultTag,
                        default.get_default_type()
                    );
                    GenericParameter::GenericTypePackWithDefault(Box::new((*default).clone()))
                }
            };
            record!(self.seen, OwnedGenericParameterTag, &owned);
        }
        parameters.for_each_child(self);
    }

    fn attributes(&mut self, attributes: &Attributes) {
        for attribute in attributes.iter_attributes() {
            record!(self.seen, AttributeTag, attribute);
            if let Attribute::Group(group) = attribute {
                for element in group.iter_attributes() {
                    if let Some(arguments) = element.get_arguments() {
                        record!(self.seen, AttributeArgumentsTag, arguments);
                    }
                }
            }
        }
        attributes.for_each_child(self);
    }

    fn table(&mut self, table: &TableExpression) {
        for entry in table.iter_entries() {
            record!(self.seen, TableEntryTag, entry);
        }
        table.for_each_child(self);
    }

    fn literal_expression(&mut self, expression: &LiteralExpression) {
        record!(self.seen, LiteralExpressionTag, expression);
        expression.for_each_child(self);
    }

    fn literal_argument(&mut self, expression: &LiteralExpression) {
        record!(self.seen, LiteralExpressionTag, expression);
        expression.for_each_child(self);
    }

    fn literal_table(&mut self, table: &LiteralTable) {
        for entry in table.iter_entries() {
            record!(self.seen, LiteralTableEntryTag, entry);
        }
        table.for_each_child(self);
    }

    fn block(&mut self, block: &Block) {
        block.for_each_child(self);
    }

    fn typed_identifier(&mut self, identifier: &TypedIdentifier) {
        identifier.for_each_child(self);
    }

    fn type_pack(&mut self, type_pack: &TypePack) {
        type_pack.for_each_child(self);
    }

    fn generic_type_pack(&mut self, generic: &GenericTypePack) {
        generic.for_each_child(self);
    }

    fn variadic_type_pack(&mut self, variadic: &VariadicTypePack) {
        variadic.for_each_child(self);
    }

    fn arguments(&mut self, arguments: &Arguments) {
        record!(self.seen, ArgumentsTag, arguments);
        arguments.for_each_child(self);
    }
}

fn parser(lua52: bool) -> Parser {
    if lua52 {
        Parser::default().lua52()
    } else {
        Parser::default()
    }
}

fn parse(code: &str, lua52: bool) -> Block {
    parser(lua52)
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"))
}

fn parse_with_tokens(code: &str, lua52: bool) -> Block {
    parser(lua52)
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}` with tokens: {error}"))
}

/// Every construct the front end can parse. Parser-unreachable variants
/// (bitwise operators, which the upstream parser refuses) are covered by the
/// constructed-node test instead.
fn corpus() -> &'static [(&'static str, bool)] {
    &[
        // statements
        ("x = 1", false),
        ("x.y = 1", false),
        ("x[1] = 1", false),
        ("do local x = 1 end", false),
        ("print(1)", false),
        ("x += 1", false),
        ("x -= 1", false),
        ("x *= 1", false),
        ("x /= 1", false),
        ("x //= 1", false),
        ("x %= 1", false),
        ("x ^= 1", false),
        ("x ..= 1", false),
        ("function f() end", false),
        ("function a.b.c() end", false),
        ("for k, v in pairs(t) do end", false),
        ("if a then elseif b then else end", false),
        ("local a = 1", false),
        ("const a = 1", false),
        ("local function f() end", false),
        ("for i = 1, 10, 2 do end", false),
        ("repeat until a", false),
        ("while a do end", false),
        ("while a do break end", false),
        ("while a do continue end", false),
        ("type T = number", false),
        ("type function f() end", false),
        ("::top::\ngoto top", true),
        // expressions and operators
        ("return a + b - c * d / e // f % g ^ h .. i", false),
        ("return a and b or c", false),
        ("return a == b, a ~= b, a < b, a <= b, a > b, a >= b", false),
        ("return f(), f(1), f 's', f { 1 }", false),
        ("return false, true, nil", false),
        ("return a.b.c", false),
        ("return function() end", false),
        ("return if a then 1 elseif b then 2 else 3", false),
        ("return a[1]", false),
        ("return 1, 0x1F, 0b101", false),
        ("return (1 + 2)", false),
        ("return 'string', [[long]]", false),
        ("return `a {b} c {d}`", false),
        ("return {}, { 1, k = 2, [x] = 3 }", false),
        ("return -a, #b, not c", false),
        ("return a :: number", false),
        ("return f<<T, U>>()", false),
        ("local function v(...) return ... end", false),
        ("return f<<T>>", false),
        ("return a[1]()", false),
        ("type A<T...> = () -> T...", false),
        ("type A<U...> = B<U...>", false),
        ("type A<T...> = (number, T...) -> ()", false),
        // prefixes and variables
        ("return f()().x[1]", false),
        ("return (f):m()", false),
        // types
        (
            "type A = number\ntype B = M.T\ntype C = true\ntype D = false\ntype E = nil",
            false,
        ),
        (
            "type A = \"x\"\ntype B = { number }\ntype C = typeof(x)",
            false,
        ),
        (
            "type A = (number)\ntype B = (number) -> string\ntype C = number?",
            false,
        ),
        ("type A = A & B\ntype B = A | B", false),
        (
            "type A = { x: number, [string]: number, [\"k\"]: number }",
            false,
        ),
        ("type A = { read x: number, write y: number }", false),
        (
            "type A = () -> number\ntype B = () -> (number, string)",
            false,
        ),
        ("type A = (number, ...string) -> ()", false),
        ("type A = () -> ...string", false),
        ("type A = B<number, (number, string), ...string>", false),
        ("type A<T, U...> = number", false),
        ("type A<T = string, U... = ...number> = number", false),
        ("type A<T... = (number, string)> = number", false),
        ("type A<G..., H... = G...> = number", false),
        // function signatures with variadic types
        ("local function f(...: number) end", false),
        ("local function f<T...>(...: T...) end", false),
        ("local function f<A>(...: A...) end", false),
        // attributes
        ("@native function f() end", false),
        // trivia
        ("#!lua\n-- comment\n return 1", false),
        // lua 5.2 labels
        ("::a::\n::b::\ngoto a", true),
    ]
}

fn assert_model_invariants(model: &r4::semantic::SemanticModel, code: &str) {
    for (index, symbol) in model.symbols.iter().enumerate() {
        assert_eq!(symbol.id.index(), index, "symbol id for `{code}`");
        assert!(
            symbol.declaring_scope.index() < model.scopes.len(),
            "declaring scope for `{code}`"
        );
    }
    for (index, scope) in model.scopes.iter().enumerate() {
        assert_eq!(scope.id.index(), index, "scope id for `{code}`");
        for &child in &scope.children {
            assert_eq!(
                model.scope(child).parent,
                Some(scope.id),
                "parent/child symmetry for `{code}`"
            );
        }
        for &symbol in &scope.symbols {
            assert_eq!(
                model.symbol(symbol).declaring_scope,
                scope.id,
                "symbol declared in its scope for `{code}`"
            );
        }
    }
}

#[test]
fn every_variant_is_reached_by_the_corpus() {
    let mut seen = BTreeSet::new();

    for &(code, lua52) in corpus() {
        let block = parse(code, lua52);
        let mut collector = TagCollector::default();
        block.for_each_child(&mut collector);
        seen.extend(collector.seen);

        let tokenful = parse_with_tokens(code, lua52);
        for trivia in collect_trivia(&tokenful) {
            record!(seen, TriviaKindTag, &trivia.kind());
        }
    }

    for (_, block) in constructed_blocks() {
        let mut collector = TagCollector::default();
        block.for_each_child(&mut collector);
        seen.extend(collector.seen);
    }

    let expected = all_tags();
    let missing: Vec<_> = expected.difference(&seen).cloned().collect();
    assert!(
        missing.is_empty(),
        "constructs registered but never reached by the corpus:\n{missing:#?}"
    );
    let unknown: Vec<_> = seen.difference(&expected).cloned().collect();
    assert!(
        unknown.is_empty(),
        "tags produced that are not registered:\n{unknown:#?}"
    );
}

#[test]
fn the_corpus_flows_through_the_whole_pipeline() {
    use r4::generator::{DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator};

    for &(code, lua52) in corpus() {
        let block = parse(code, lua52);

        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .unwrap_or_else(|error| panic!("analysis of `{code}` failed: {error}"));
        assert_model_invariants(&model, code);

        let cfg = build_chunk(&block, &model)
            .unwrap_or_else(|error| panic!("CFG of `{code}` failed: {error}"));
        cfg.verify_invariants()
            .unwrap_or_else(|error| panic!("CFG invariants of `{code}` failed: {error:?}"));
        Liveness::analyze(&cfg, &model)
            .unwrap_or_else(|error| panic!("liveness of `{code}` failed: {error}"));
        ConstantPropagation::analyze(&cfg, &model)
            .unwrap_or_else(|error| panic!("constant propagation of `{code}` failed: {error}"));

        let mut readable = ReadableLuaGenerator::default();
        readable.write_block(&block);
        let mut dense = DenseLuaGenerator::default();
        dense.write_block(&block);

        for (name, output) in [
            ("readable", readable.into_string()),
            ("dense", dense.into_string()),
        ] {
            let reparsed = parser(lua52).parse(&output).unwrap_or_else(|error| {
                panic!("{name} output of `{code}` does not parse: {error}\n{output}")
            });
            assert!(
                block.is_equivalent_to(&reparsed),
                "{name} output of `{code}` is not equivalent:\n{output}"
            );
        }
    }
}

/// Constructs the variants the upstream parser cannot produce: the bitwise
/// operators it panics on, and attribute groups with arguments (full_moon only
/// parses a single `@name`). These are built with r4's own node API and pushed
/// through analysis, the CFG and generation; they are not re-parsed because the
/// parser cannot read them back.
fn constructed_blocks() -> Vec<(&'static str, Block)> {
    let mut blocks = Vec::new();

    for (name, operator) in [
        ("binary BitwiseOr", BinaryOperator::BitwiseOr),
        ("binary BitwiseXor", BinaryOperator::BitwiseXor),
        ("binary BitwiseAnd", BinaryOperator::BitwiseAnd),
        ("binary ShiftLeft", BinaryOperator::ShiftLeft),
        ("binary ShiftRight", BinaryOperator::ShiftRight),
    ] {
        let binary = BinaryExpression::new(operator, Expression::from(1), Expression::from(2));
        let last = LastStatement::Return(ReturnStatement::one(Expression::Binary(binary.into())));
        blocks.push((name, Block::new(Vec::new(), Some(last))));
    }

    let unary = UnaryExpression::new(UnaryOperator::BitwiseNot, Expression::from(1));
    let last = LastStatement::Return(ReturnStatement::one(Expression::Unary(unary.into())));
    blocks.push(("unary BitwiseNot", Block::new(Vec::new(), Some(last))));

    let mut literal = LiteralTable::from_entries(Vec::new());
    literal.append_field(Identifier::new("k"), LiteralExpression::nil());
    literal.append_array_value(LiteralExpression::from(StringExpression::from_value(
        "deep",
    )));

    let mut tuple = AttributeTupleArguments::default();
    tuple.push(LiteralExpression::from(StringExpression::from_value(
        "tuple-string",
    )));
    tuple.push(LiteralExpression::from(literal));
    tuple.push(LiteralExpression::from(true));
    tuple.push(LiteralExpression::from(false));
    tuple.push(LiteralExpression::from(DecimalNumber::new(1.0)));

    let group = AttributeGroup::new(
        AttributeGroupElement::new("tuple").with_arguments(AttributeArguments::Tuple(tuple)),
    )
    .with_attribute(
        AttributeGroupElement::new("table")
            .with_arguments(AttributeArguments::Table(LiteralTable::default())),
    )
    .with_attribute(AttributeGroupElement::new("string").with_arguments(
        AttributeArguments::String(StringExpression::from_value("arg")),
    ));

    let function = FunctionStatement::from_name("f", Block::default())
        .with_attributes(Attributes::new().with_attribute(Attribute::Group(group)));
    blocks.push((
        "attribute group with arguments",
        Block::new(vec![Statement::Function(Box::new(function))], None),
    ));

    blocks
}

#[test]
fn constructed_constructs_flow_through_the_pipeline() {
    use r4::generator::{LuaGenerator, ReadableLuaGenerator};

    for (name, block) in constructed_blocks() {
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .unwrap_or_else(|error| panic!("analysis of `{name}` failed: {error}"));
        assert_model_invariants(&model, name);

        let cfg = build_chunk(&block, &model)
            .unwrap_or_else(|error| panic!("CFG of `{name}` failed: {error}"));
        cfg.verify_invariants()
            .unwrap_or_else(|error| panic!("CFG invariants of `{name}` failed: {error:?}"));

        let mut readable = ReadableLuaGenerator::default();
        readable.write_block(&block);
        assert!(
            !readable.into_string().is_empty(),
            "generation of `{name}` produced nothing"
        );
    }
}

trait RuleOutcome {
    fn ensure_ok(self, pass: &str, code: &str);
}

impl RuleOutcome for () {
    fn ensure_ok(self, _pass: &str, _code: &str) {}
}

impl RuleOutcome for usize {
    fn ensure_ok(self, _pass: &str, _code: &str) {}
}

impl<T, E: std::fmt::Display> RuleOutcome for Result<T, E> {
    fn ensure_ok(self, pass: &str, code: &str) {
        if let Err(error) = self {
            panic!("`{pass}` failed on `{code}`: {error}");
        }
    }
}

#[test]
fn the_corpus_is_run_through_every_rule() {
    use std::collections::HashMap;

    use r4::cfg::build_chunk;
    use r4::generator::{LuaGenerator, ReadableLuaGenerator};
    use r4::modules::MemoryModuleProvider;
    use r4::nodes::{Expression, StringExpression};
    use r4::rules::require::{
        apply_path_aliases, bundle_modules, convert_require, RequirePathConverter,
    };
    use r4::rules::{canonicalize, desugar, inject, rename_variables};

    struct Echo;

    impl RequirePathConverter for Echo {
        fn convert(&self, path: &str) -> Option<Expression> {
            Some(Expression::String(StringExpression::from_value(path)))
        }
    }

    macro_rules! rule {
        ($code:expr, $lua52:expr, $block:ident, $model:expr, $pass:expr) => {{
            let code = $code;
            let lua52 = $lua52;
            let mut applied = $block.clone();
            RuleOutcome::ensure_ok(($pass)(&mut applied, $model), stringify!($pass), code);

            let mut generator = ReadableLuaGenerator::default();
            generator.write_block(&applied);
            let output = generator.into_string();
            let reparsed = parser(lua52).parse(&output).unwrap_or_else(|error| {
                panic!(
                    "`{}` output of `{code}` does not parse: {error}\n{output}",
                    stringify!($pass)
                )
            });
            let model = SemanticAnalyzer::new()
                .analyze(&reparsed)
                .unwrap_or_else(|error| {
                    panic!(
                        "`{}` output of `{code}` does not analyze: {error}",
                        stringify!($pass)
                    )
                });
            let cfg = build_chunk(&reparsed, &model).unwrap_or_else(|error| {
                panic!(
                    "`{}` output of `{code}` does not lower: {error}",
                    stringify!($pass)
                )
            });
            cfg.verify_invariants().unwrap_or_else(|error| {
                panic!(
                    "`{}` output of `{code}` has an invalid CFG: {error:?}",
                    stringify!($pass)
                )
            });
        }};
    }

    for &(code, lua52) in corpus() {
        let block = parse(code, lua52);
        let model = SemanticAnalyzer::new()
            .analyze(&block)
            .unwrap_or_else(|error| panic!("analysis of `{code}` failed: {error}"));
        let empty_values: HashMap<String, r4::process::LuaValue> = HashMap::new();

        rule!(
            code,
            lua52,
            block,
            &model,
            canonicalize::convert_index_to_field
        );
        rule!(
            code,
            lua52,
            block,
            &model,
            canonicalize::remove_nil_declarations
        );
        rule!(code, lua52, block, &model, canonicalize::remove_empty_do);
        rule!(code, lua52, block, &model, canonicalize::remove_call_parens);
        rule!(code, lua52, block, &model, canonicalize::compute_expression);
        rule!(code, lua52, block, &model, canonicalize::remove_assertions);
        rule!(
            code,
            lua52,
            block,
            &model,
            canonicalize::remove_debug_profiling
        );
        rule!(
            code,
            lua52,
            block,
            &model,
            canonicalize::remove_unused_variables
        );
        rule!(
            code,
            lua52,
            block,
            &model,
            canonicalize::remove_unused_labels
        );
        rule!(code, lua52, block, &model, canonicalize::remove_dead_stores);
        rule!(
            code,
            lua52,
            block,
            &model,
            canonicalize::remove_unreachable_code
        );
        rule!(code, lua52, block, &model, canonicalize::group_local);
        rule!(
            code,
            lua52,
            block,
            &model,
            canonicalize::hoist_loop_invariants
        );
        rule!(code, lua52, block, &model, desugar::remove_types);
        rule!(code, lua52, block, &model, desugar::remove_compound_assign);
        rule!(code, lua52, block, &model, desugar::remove_if_expression);
        rule!(
            code,
            lua52,
            block,
            &model,
            desugar::remove_interpolated_string
        );
        rule!(code, lua52, block, &model, desugar::remove_floor_division);
        rule!(code, lua52, block, &model, desugar::remove_continue);
        rule!(code, lua52, block, &model, desugar::convert_luau_number);
        rule!(code, lua52, block, &model, desugar::remove_method_call);
        rule!(code, lua52, block, &model, desugar::make_assignment_local);
        rule!(code, lua52, block, &model, desugar::remove_attribute);
        rule!(
            code,
            lua52,
            block,
            &model,
            desugar::remove_method_definition
        );
        rule!(
            code,
            lua52,
            block,
            &model,
            desugar::convert_function_to_assignment
        );
        rule!(
            code,
            lua52,
            block,
            &model,
            desugar::convert_local_function_to_assignment
        );
        rule!(
            code,
            lua52,
            block,
            &model,
            rename_variables::rename_variables
        );
        rule!(code, lua52, block, &model, |b, m| {
            inject::inject_global_values(b, m, &empty_values)
        });
        rule!(code, lua52, block, &model, |b, m| convert_require(
            b, m, &Echo
        ));
        rule!(code, lua52, block, &model, |b, m| apply_path_aliases(
            b,
            m,
            [("@pkg", "src/pkg")]
        ));
        rule!(code, lua52, block, &model, |b, m| bundle_modules(
            b,
            m,
            &MemoryModuleProvider::new()
        ));
    }
}
