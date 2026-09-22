//! Read-only, result-carrying traversal of the syntax tree.
//!
//! [`ImmutableVisitor`] walks a borrowed tree and drives an
//! [`ImmutableProcessor`] through its hooks. Unlike
//! [`NodeProcessor`](crate::process::NodeProcessor), it never mutates a node and
//! never clones the tree to inspect it, and the processor accumulates a result
//! across the traversal (a result-carrying traversal).
//!
//! The traversal covers every statement, expression, prefix, variable, type and
//! function signature, so a processor cannot miss a node because a kind was not
//! visited. The child structure itself is defined once in
//! [`crate::nodes::ReadOnlyChildren`].

use std::marker::PhantomData;

use crate::nodes::*;

/// A concrete [`ImmutableVisitor`] for callers that do not need their own type.
pub struct Walk;

impl ImmutableVisitor for Walk {}

/// A read-only hook set driven by [`ImmutableVisitor`].
///
/// Every callback receives a shared borrow and returns nothing; the processor
/// owns the accumulated result. Implementations may override any subset.
pub trait ImmutableProcessor {
    fn process_block(&mut self, _: &Block) {}
    fn process_statement(&mut self, _: &Statement) {}
    fn process_last_statement(&mut self, _: &LastStatement) {}
    fn process_expression(&mut self, _: &Expression) {}
    fn process_prefix(&mut self, _: &Prefix) {}
    fn process_variable(&mut self, _: &Variable) {}
    fn process_identifier(&mut self, _: &Identifier) {}
    fn process_function_call(&mut self, _: &FunctionCall) {}
    fn process_table(&mut self, _: &TableExpression) {}
    fn process_typed_identifier(&mut self, _: &TypedIdentifier) {}
    fn process_attributes(&mut self, _: &Attributes) {}
    fn process_type(&mut self, _: &Type) {}
    fn process_type_name(&mut self, _: &TypeName) {}
    fn process_literal_expression(&mut self, _: &LiteralExpression) {}
    fn process_literal_table(&mut self, _: &LiteralTable) {}
    fn process_number_expression(&mut self, _: &NumberExpression) {}
    fn process_string_expression(&mut self, _: &StringExpression) {}
}

/// Adapts an [`ImmutableVisitor`] to the child visitor used by the
/// authoritative child structure in [`crate::nodes::ReadOnlyChildren`].
struct ImmutableChildDispatch<'processor, P, V>
where
    P: ImmutableProcessor,
    V: ImmutableVisitor + ?Sized,
{
    processor: &'processor mut P,
    _visitor: PhantomData<V>,
}

impl<P, V> ChildVisitor for ImmutableChildDispatch<'_, P, V>
where
    P: ImmutableProcessor,
    V: ImmutableVisitor + ?Sized,
{
    fn block(&mut self, block: &Block) {
        V::visit_block(block, self.processor);
    }

    fn statement(&mut self, statement: &Statement) {
        V::visit_statement(statement, self.processor);
    }

    fn last_statement(&mut self, last_statement: &LastStatement) {
        V::visit_last_statement(last_statement, self.processor);
    }

    fn expression(&mut self, expression: &Expression) {
        V::visit_expression(expression, self.processor);
    }

    fn prefix(&mut self, prefix: &Prefix) {
        V::visit_prefix(prefix, self.processor);
    }

    fn variable(&mut self, variable: &Variable) {
        V::visit_variable(variable, self.processor);
    }

    fn identifier(&mut self, identifier: &Identifier) {
        V::visit_identifier(identifier, self.processor);
    }

    fn variable_expression(&mut self, identifier: &Identifier) {
        V::visit_identifier(identifier, self.processor);
    }

    fn attributes(&mut self, attributes: &Attributes) {
        V::visit_attributes(attributes, self.processor);
    }

    fn typed_identifier(&mut self, identifier: &TypedIdentifier) {
        V::visit_typed_identifier(identifier, self.processor);
    }

    fn r#type(&mut self, r#type: &Type) {
        V::visit_type(r#type, self.processor);
    }

    fn type_name(&mut self, type_name: &TypeName) {
        V::visit_type_name(type_name, self.processor);
    }

    fn type_pack(&mut self, type_pack: &TypePack) {
        V::visit_type_pack(type_pack, self.processor);
    }

    fn generic_type_pack(&mut self, generic_type_pack: &GenericTypePack) {
        V::visit_generic_type_pack(generic_type_pack, self.processor);
    }

    fn variadic_type_pack(&mut self, variadic_type_pack: &VariadicTypePack) {
        V::visit_variadic_type_pack(variadic_type_pack, self.processor);
    }

    fn variadic_argument_type(&mut self, argument: &VariadicArgumentType) {
        V::visit_variadic_argument_type(argument, self.processor);
    }

    fn function_variadic_type(&mut self, variadic: &FunctionVariadicType) {
        V::visit_function_variadic_type(variadic, self.processor);
    }

    fn function_return_type(&mut self, r#type: &FunctionReturnType) {
        V::visit_function_return_type(r#type, self.processor);
    }

    fn generic_parameters(&mut self, parameters: &GenericParameters) {
        V::visit_generic_parameters(Some(parameters), self.processor);
    }

    fn generic_parameters_with_defaults(&mut self, parameters: &GenericParametersWithDefaults) {
        V::visit_generic_parameters_with_defaults(Some(parameters), self.processor);
    }

    fn table(&mut self, table: &TableExpression) {
        V::visit_table(table, self.processor);
    }

    fn arguments(&mut self, arguments: &Arguments) {
        V::visit_arguments(arguments, self.processor);
    }

    fn literal_expression(&mut self, expression: &LiteralExpression) {
        V::visit_literal_expression(expression, self.processor);
    }

    fn literal_argument(&mut self, expression: &LiteralExpression) {
        V::visit_literal_expression(expression, self.processor);
    }

    fn literal_table(&mut self, table: &LiteralTable) {
        V::visit_literal_table(table, self.processor);
    }

    fn number_expression(&mut self, number: &NumberExpression) {
        V::visit_number_expression(number, self.processor);
    }

    fn string_expression(&mut self, string: &StringExpression) {
        V::visit_string_expression(string, self.processor);
    }
}

/// A read-only traversal driver. The default methods recurse through the whole
/// tree; call [`ImmutableVisitor::visit_block`] to start.
pub trait ImmutableVisitor {
    /// Recurses into the children of `node` through the single authoritative
    /// child definition, dispatching each child to the matching `visit_*`
    /// method.
    fn visit_children<C, P>(node: &C, processor: &mut P)
    where
        C: ReadOnlyChildren,
        P: ImmutableProcessor,
    {
        let mut dispatch = ImmutableChildDispatch::<P, Self> {
            processor,
            _visitor: PhantomData,
        };
        node.for_each_child(&mut dispatch);
    }

    fn visit_block(block: &Block, processor: &mut impl ImmutableProcessor) {
        processor.process_block(block);
        Self::visit_children(block, processor);
    }

    fn visit_statement(statement: &Statement, processor: &mut impl ImmutableProcessor) {
        processor.process_statement(statement);
        match statement {
            Statement::Assign(statement) => Self::visit_children(statement, processor),
            Statement::Do(statement) => Self::visit_children(statement, processor),
            Statement::Call(call) => Self::visit_function_call(call, processor),
            Statement::CompoundAssign(statement) => Self::visit_children(statement, processor),
            Statement::Function(function) => Self::visit_children(&**function, processor),
            Statement::GenericFor(statement) => Self::visit_children(statement, processor),
            Statement::Goto(statement) => Self::visit_children(statement, processor),
            Statement::If(statement) => Self::visit_children(statement, processor),
            Statement::Label(statement) => Self::visit_children(statement, processor),
            Statement::LocalAssign(statement) => Self::visit_children(statement, processor),
            Statement::LocalFunction(function) => Self::visit_children(&**function, processor),
            Statement::NumericFor(statement) => Self::visit_children(&**statement, processor),
            Statement::Repeat(statement) => Self::visit_children(statement, processor),
            Statement::While(statement) => Self::visit_children(statement, processor),
            Statement::TypeDeclaration(statement) => Self::visit_children(statement, processor),
            Statement::TypeFunction(function) => Self::visit_children(&**function, processor),
        }
    }

    fn visit_last_statement(
        last_statement: &LastStatement,
        processor: &mut impl ImmutableProcessor,
    ) {
        processor.process_last_statement(last_statement);
        Self::visit_children(last_statement, processor);
    }

    fn visit_expression(expression: &Expression, processor: &mut impl ImmutableProcessor) {
        processor.process_expression(expression);
        match expression {
            Expression::Binary(binary) => Self::visit_children(&**binary, processor),
            Expression::Call(call) => Self::visit_function_call(call, processor),
            Expression::Field(field) => Self::visit_children(&**field, processor),
            Expression::Function(function) => Self::visit_children(&**function, processor),
            Expression::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Expression::If(if_expression) => Self::visit_children(&**if_expression, processor),
            Expression::Index(index) => Self::visit_children(&**index, processor),
            Expression::InterpolatedString(string) => Self::visit_children(string, processor),
            Expression::Number(number) => Self::visit_number_expression(number, processor),
            Expression::Parenthese(parenthese) => Self::visit_children(&**parenthese, processor),
            Expression::String(string) => Self::visit_string_expression(string, processor),
            Expression::Table(table) => Self::visit_table(table, processor),
            Expression::Unary(unary) => Self::visit_children(&**unary, processor),
            Expression::TypeCast(type_cast) => Self::visit_children(type_cast, processor),
            Expression::TypeInstantiation(instantiation) => {
                Self::visit_children(&**instantiation, processor)
            }
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => {}
        }
    }

    fn visit_prefix(prefix: &Prefix, processor: &mut impl ImmutableProcessor) {
        processor.process_prefix(prefix);
        match prefix {
            Prefix::Call(call) => Self::visit_function_call(call, processor),
            Prefix::Field(field) => Self::visit_children(&**field, processor),
            Prefix::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Prefix::Index(index) => Self::visit_children(&**index, processor),
            Prefix::Parenthese(parenthese) => Self::visit_children(&**parenthese, processor),
            Prefix::TypeInstantiation(instantiation) => {
                Self::visit_children(&**instantiation, processor)
            }
        }
    }

    fn visit_variable(variable: &Variable, processor: &mut impl ImmutableProcessor) {
        processor.process_variable(variable);
        match variable {
            Variable::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Variable::Field(field) => Self::visit_children(&**field, processor),
            Variable::Index(index) => Self::visit_children(&**index, processor),
        }
    }

    fn visit_identifier(identifier: &Identifier, processor: &mut impl ImmutableProcessor) {
        processor.process_identifier(identifier);
    }

    fn visit_function_call(call: &FunctionCall, processor: &mut impl ImmutableProcessor) {
        processor.process_function_call(call);
        Self::visit_children(call, processor);
    }

    fn visit_arguments(arguments: &Arguments, processor: &mut impl ImmutableProcessor) {
        Self::visit_children(arguments, processor);
    }

    fn visit_table(table: &TableExpression, processor: &mut impl ImmutableProcessor) {
        processor.process_table(table);
        Self::visit_children(table, processor);
    }

    fn visit_typed_identifier(
        identifier: &TypedIdentifier,
        processor: &mut impl ImmutableProcessor,
    ) {
        processor.process_typed_identifier(identifier);
        Self::visit_children(identifier, processor);
    }

    fn visit_number_expression(number: &NumberExpression, processor: &mut impl ImmutableProcessor) {
        processor.process_number_expression(number);
    }

    fn visit_string_expression(string: &StringExpression, processor: &mut impl ImmutableProcessor) {
        processor.process_string_expression(string);
    }

    fn visit_literal_table(table: &LiteralTable, processor: &mut impl ImmutableProcessor) {
        processor.process_literal_table(table);
        Self::visit_children(table, processor);
    }

    fn visit_literal_expression(
        expression: &LiteralExpression,
        processor: &mut impl ImmutableProcessor,
    ) {
        processor.process_literal_expression(expression);
        Self::visit_children(expression, processor);
    }

    fn visit_attributes(attributes: &Attributes, processor: &mut impl ImmutableProcessor) {
        processor.process_attributes(attributes);
        Self::visit_children(attributes, processor);
    }

    fn visit_function_variadic_type(
        variadic_type: &FunctionVariadicType,
        processor: &mut impl ImmutableProcessor,
    ) {
        Self::visit_children(variadic_type, processor);
    }

    fn visit_function_return_type(
        return_type: &FunctionReturnType,
        processor: &mut impl ImmutableProcessor,
    ) {
        Self::visit_children(return_type, processor);
    }

    fn visit_type_pack(type_pack: &TypePack, processor: &mut impl ImmutableProcessor) {
        Self::visit_children(type_pack, processor);
    }

    fn visit_variadic_argument_type(
        variadic: &VariadicArgumentType,
        processor: &mut impl ImmutableProcessor,
    ) {
        Self::visit_children(variadic, processor);
    }

    fn visit_variadic_type_pack(
        variadic: &VariadicTypePack,
        processor: &mut impl ImmutableProcessor,
    ) {
        Self::visit_children(variadic, processor);
    }

    fn visit_generic_type_pack(generic: &GenericTypePack, processor: &mut impl ImmutableProcessor) {
        Self::visit_children(generic, processor);
    }

    fn visit_generic_parameters(
        generic_parameters: Option<&GenericParameters>,
        processor: &mut impl ImmutableProcessor,
    ) {
        let Some(generic_parameters) = generic_parameters else {
            return;
        };
        Self::visit_children(generic_parameters, processor);
    }

    fn visit_generic_parameters_with_defaults(
        generic_parameters: Option<&GenericParametersWithDefaults>,
        processor: &mut impl ImmutableProcessor,
    ) {
        let Some(generic_parameters) = generic_parameters else {
            return;
        };
        Self::visit_children(generic_parameters, processor);
    }

    fn visit_type(r#type: &Type, processor: &mut impl ImmutableProcessor) {
        processor.process_type(r#type);
        match r#type {
            Type::Name(type_name) => Self::visit_type_name(type_name, processor),
            Type::Field(field) => Self::visit_children(&**field, processor),
            Type::True(_) | Type::False(_) | Type::Nil(_) | Type::String(_) => {}
            Type::Array(array) => Self::visit_children(array, processor),
            Type::Table(table) => Self::visit_children(table, processor),
            Type::TypeOf(expression_type) => Self::visit_children(expression_type, processor),
            Type::Parenthese(parenthese) => Self::visit_children(parenthese, processor),
            Type::Function(function) => Self::visit_children(&**function, processor),
            Type::Optional(optional) => Self::visit_children(optional, processor),
            Type::Intersection(intersection) => Self::visit_children(intersection, processor),
            Type::Union(union) => Self::visit_children(union, processor),
        }
    }

    fn visit_type_name(type_name: &TypeName, processor: &mut impl ImmutableProcessor) {
        processor.process_type_name(type_name);
        Self::visit_children(type_name, processor);
    }
}

/// Returns whether `block` contains an expression accepted by `predicate`.
pub fn any_expression(block: &Block, predicate: impl FnMut(&Expression) -> bool) -> bool {
    struct Finder<F> {
        predicate: F,
        found: bool,
    }

    impl<F: FnMut(&Expression) -> bool> ImmutableProcessor for Finder<F> {
        fn process_expression(&mut self, expression: &Expression) {
            if !self.found && (self.predicate)(expression) {
                self.found = true;
            }
        }
    }

    let mut finder = Finder {
        predicate,
        found: false,
    };
    Walk::visit_block(block, &mut finder);
    finder.found
}

/// Returns whether `block` contains a statement accepted by `predicate`.
pub fn any_statement(block: &Block, predicate: impl FnMut(&Statement) -> bool) -> bool {
    struct Finder<F> {
        predicate: F,
        found: bool,
    }

    impl<F: FnMut(&Statement) -> bool> ImmutableProcessor for Finder<F> {
        fn process_statement(&mut self, statement: &Statement) {
            if !self.found && (self.predicate)(statement) {
                self.found = true;
            }
        }
    }

    let mut finder = Finder {
        predicate,
        found: false,
    };
    Walk::visit_block(block, &mut finder);
    finder.found
}

/// Collects the name of every identifier occurrence, in source order.
pub fn collect_identifier_names(block: &Block) -> Vec<String> {
    #[derive(Default)]
    struct Collector {
        names: Vec<String>,
    }

    impl ImmutableProcessor for Collector {
        fn process_identifier(&mut self, identifier: &Identifier) {
            self.names.push(identifier.get_name().clone());
        }
    }

    let mut collector = Collector::default();
    Walk::visit_block(block, &mut collector);
    collector.names
}

/// Counts the statements and expressions in `block`.
pub fn count_nodes(block: &Block) -> (usize, usize) {
    #[derive(Default)]
    struct Counter {
        statements: usize,
        expressions: usize,
    }

    impl ImmutableProcessor for Counter {
        fn process_statement(&mut self, _: &Statement) {
            self.statements += 1;
        }

        fn process_expression(&mut self, _: &Expression) {
            self.expressions += 1;
        }
    }

    let mut counter = Counter::default();
    Walk::visit_block(block, &mut counter);
    (counter.statements, counter.expressions)
}
