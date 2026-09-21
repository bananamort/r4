//! Read-only, result-carrying traversal of the syntax tree.
//!
//! [`ImmutableVisitor`] walks a borrowed tree and drives an
//! [`ImmutableProcessor`] through its hooks. Unlike
//! [`NodeProcessor`](crate::process::NodeProcessor), it never mutates a node and
//! never clones the tree to inspect it, and the processor accumulates a result
//! across the traversal (the result-carrying counterpart of Loretta's
//! `LuaSyntaxVisitor<TResult>`).
//!
//! The traversal covers every statement, expression, prefix, variable, type and
//! function signature, so a processor cannot miss a node because a kind was not
//! visited.

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

/// A read-only traversal driver. The default methods recurse through the whole
/// tree; call [`ImmutableVisitor::visit_block`] to start.
pub trait ImmutableVisitor {
    fn visit_block(block: &Block, processor: &mut impl ImmutableProcessor) {
        processor.process_block(block);
        for statement in block.iter_statements() {
            Self::visit_statement(statement, processor);
        }
        if let Some(last_statement) = block.get_last_statement() {
            Self::visit_last_statement(last_statement, processor);
        }
    }

    fn visit_statement(statement: &Statement, processor: &mut impl ImmutableProcessor) {
        processor.process_statement(statement);
        match statement {
            Statement::Assign(statement) => {
                for variable in statement.iter_variables() {
                    Self::visit_variable(variable, processor);
                }
                for value in statement.iter_values() {
                    Self::visit_expression(value, processor);
                }
            }
            Statement::Do(statement) => {
                Self::visit_block(statement.get_block(), processor);
            }
            Statement::Call(call) => Self::visit_function_call(call, processor),
            Statement::CompoundAssign(statement) => {
                Self::visit_variable(statement.get_variable(), processor);
                Self::visit_expression(statement.get_value(), processor);
            }
            Statement::Function(function) => {
                Self::visit_attributes(function.attributes(), processor);
                Self::visit_identifier(function.get_name().get_name(), processor);
                for field in function.get_name().get_field_names() {
                    Self::visit_identifier(field, processor);
                }
                if let Some(method) = function.get_name().get_method() {
                    Self::visit_identifier(method, processor);
                }
                Self::visit_function_signature(
                    function.get_generic_parameters(),
                    function.iter_parameters(),
                    function.get_variadic_type(),
                    function.get_return_type(),
                    processor,
                );
                Self::visit_block(function.get_block(), processor);
            }
            Statement::GenericFor(statement) => {
                for expression in statement.iter_expressions() {
                    Self::visit_expression(expression, processor);
                }
                for identifier in statement.iter_identifiers() {
                    Self::visit_typed_identifier(identifier, processor);
                }
                Self::visit_block(statement.get_block(), processor);
            }
            Statement::Goto(statement) => Self::visit_identifier(statement.get_label(), processor),
            Statement::Label(statement) => Self::visit_identifier(statement.get_label(), processor),
            Statement::If(statement) => {
                for branch in statement.iter_branches() {
                    Self::visit_expression(branch.get_condition(), processor);
                    Self::visit_block(branch.get_block(), processor);
                }
                if let Some(else_block) = statement.get_else_block() {
                    Self::visit_block(else_block, processor);
                }
            }
            Statement::LocalAssign(statement) => {
                for variable in statement.iter_variables() {
                    Self::visit_typed_identifier(variable, processor);
                }
                for value in statement.iter_values() {
                    Self::visit_expression(value, processor);
                }
            }
            Statement::LocalFunction(function) => {
                Self::visit_attributes(function.attributes(), processor);
                Self::visit_identifier(function.get_identifier(), processor);
                Self::visit_function_signature(
                    function.get_generic_parameters(),
                    function.iter_parameters(),
                    function.get_variadic_type(),
                    function.get_return_type(),
                    processor,
                );
                Self::visit_block(function.get_block(), processor);
            }
            Statement::NumericFor(statement) => {
                Self::visit_expression(statement.get_start(), processor);
                Self::visit_expression(statement.get_end(), processor);
                if let Some(step) = statement.get_step() {
                    Self::visit_expression(step, processor);
                }
                Self::visit_typed_identifier(statement.get_identifier(), processor);
                Self::visit_block(statement.get_block(), processor);
            }
            Statement::Repeat(statement) => {
                Self::visit_block(statement.get_block(), processor);
                Self::visit_expression(statement.get_condition(), processor);
            }
            Statement::While(statement) => {
                Self::visit_expression(statement.get_condition(), processor);
                Self::visit_block(statement.get_block(), processor);
            }
            Statement::TypeDeclaration(statement) => {
                Self::visit_identifier(statement.get_name(), processor);
                Self::visit_generic_parameters_with_defaults(
                    statement.get_generic_parameters(),
                    processor,
                );
                Self::visit_type(statement.get_type(), processor);
            }
            Statement::TypeFunction(function) => {
                Self::visit_function_signature(
                    function.get_generic_parameters(),
                    function.iter_parameters(),
                    function.get_variadic_type(),
                    function.get_return_type(),
                    processor,
                );
                Self::visit_block(function.get_block(), processor);
            }
        }
    }

    fn visit_last_statement(
        last_statement: &LastStatement,
        processor: &mut impl ImmutableProcessor,
    ) {
        processor.process_last_statement(last_statement);
        if let LastStatement::Return(statement) = last_statement {
            for expression in statement.iter_expressions() {
                Self::visit_expression(expression, processor);
            }
        }
    }

    fn visit_expression(expression: &Expression, processor: &mut impl ImmutableProcessor) {
        processor.process_expression(expression);
        match expression {
            Expression::Binary(binary) => {
                Self::visit_expression(binary.left(), processor);
                Self::visit_expression(binary.right(), processor);
            }
            Expression::Call(call) => Self::visit_function_call(call, processor),
            Expression::Field(field) => {
                Self::visit_prefix(field.get_prefix(), processor);
                Self::visit_identifier(field.get_field(), processor);
            }
            Expression::Function(function) => {
                Self::visit_attributes(function.attributes(), processor);
                Self::visit_function_signature(
                    function.get_generic_parameters(),
                    function.iter_parameters(),
                    function.get_variadic_type(),
                    function.get_return_type(),
                    processor,
                );
                Self::visit_block(function.get_block(), processor);
            }
            Expression::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Expression::If(if_expression) => {
                Self::visit_expression(if_expression.get_condition(), processor);
                Self::visit_expression(if_expression.get_result(), processor);
                for branch in if_expression.iter_branches() {
                    Self::visit_expression(branch.get_condition(), processor);
                    Self::visit_expression(branch.get_result(), processor);
                }
                Self::visit_expression(if_expression.get_else_result(), processor);
            }
            Expression::Index(index) => {
                Self::visit_prefix(index.get_prefix(), processor);
                Self::visit_expression(index.get_index(), processor);
            }
            Expression::InterpolatedString(string) => {
                for segment in string.iter_segments() {
                    if let InterpolationSegment::Value(value) = segment {
                        Self::visit_expression(value.get_expression(), processor);
                    }
                }
            }
            Expression::Parenthese(parenthese) => {
                Self::visit_expression(parenthese.inner_expression(), processor);
            }
            Expression::Table(table) => Self::visit_table(table, processor),
            Expression::Unary(unary) => Self::visit_expression(unary.get_expression(), processor),
            Expression::TypeCast(type_cast) => {
                Self::visit_expression(type_cast.get_expression(), processor);
                Self::visit_type(type_cast.get_type(), processor);
            }
            Expression::TypeInstantiation(instantiation) => {
                Self::visit_prefix(instantiation.get_prefix(), processor);
                for r#type in instantiation.iter_types() {
                    Self::visit_type(r#type, processor);
                }
            }
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::String(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => {}
        }
    }

    fn visit_prefix(prefix: &Prefix, processor: &mut impl ImmutableProcessor) {
        processor.process_prefix(prefix);
        match prefix {
            Prefix::Call(call) => Self::visit_function_call(call, processor),
            Prefix::Field(field) => {
                Self::visit_prefix(field.get_prefix(), processor);
                Self::visit_identifier(field.get_field(), processor);
            }
            Prefix::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Prefix::Index(index) => {
                Self::visit_prefix(index.get_prefix(), processor);
                Self::visit_expression(index.get_index(), processor);
            }
            Prefix::Parenthese(parenthese) => {
                Self::visit_expression(parenthese.inner_expression(), processor);
            }
            Prefix::TypeInstantiation(instantiation) => {
                Self::visit_prefix(instantiation.get_prefix(), processor);
                for r#type in instantiation.iter_types() {
                    Self::visit_type(r#type, processor);
                }
            }
        }
    }

    fn visit_variable(variable: &Variable, processor: &mut impl ImmutableProcessor) {
        processor.process_variable(variable);
        match variable {
            Variable::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Variable::Field(field) => {
                Self::visit_prefix(field.get_prefix(), processor);
                Self::visit_identifier(field.get_field(), processor);
            }
            Variable::Index(index) => {
                Self::visit_prefix(index.get_prefix(), processor);
                Self::visit_expression(index.get_index(), processor);
            }
        }
    }

    fn visit_identifier(identifier: &Identifier, processor: &mut impl ImmutableProcessor) {
        processor.process_identifier(identifier);
    }

    fn visit_function_call(call: &FunctionCall, processor: &mut impl ImmutableProcessor) {
        processor.process_function_call(call);
        Self::visit_prefix(call.get_prefix(), processor);
        if let Some(method) = call.get_method() {
            Self::visit_identifier(method, processor);
        }
        Self::visit_arguments(call.get_arguments(), processor);
    }

    fn visit_arguments(arguments: &Arguments, processor: &mut impl ImmutableProcessor) {
        match arguments {
            Arguments::Tuple(tuple) => {
                for value in tuple.iter_values() {
                    Self::visit_expression(value, processor);
                }
            }
            Arguments::String(_) => {}
            Arguments::Table(table) => Self::visit_table(table, processor),
        }
    }

    fn visit_table(table: &TableExpression, processor: &mut impl ImmutableProcessor) {
        processor.process_table(table);
        for entry in table.iter_entries() {
            match entry {
                TableEntry::Field(field) => {
                    Self::visit_identifier(field.get_field(), processor);
                    Self::visit_expression(field.get_value(), processor);
                }
                TableEntry::Index(index) => {
                    Self::visit_expression(index.get_key(), processor);
                    Self::visit_expression(index.get_value(), processor);
                }
                TableEntry::Value(value) => Self::visit_expression(value, processor),
            }
        }
    }

    fn visit_typed_identifier(
        identifier: &TypedIdentifier,
        processor: &mut impl ImmutableProcessor,
    ) {
        processor.process_typed_identifier(identifier);
        Self::visit_identifier(identifier.get_identifier(), processor);
        if let Some(r#type) = identifier.get_type() {
            Self::visit_type(r#type, processor);
        }
    }

    fn visit_number_expression(number: &NumberExpression, processor: &mut impl ImmutableProcessor) {
        processor.process_number_expression(number);
    }

    fn visit_string_expression(string: &StringExpression, processor: &mut impl ImmutableProcessor) {
        processor.process_string_expression(string);
    }

    fn visit_literal_table(table: &LiteralTable, processor: &mut impl ImmutableProcessor) {
        processor.process_literal_table(table);
        for entry in table.iter_entries() {
            match entry {
                LiteralTableEntry::Field(field) => {
                    Self::visit_identifier(field.get_field(), processor);
                    Self::visit_literal_expression(field.get_value(), processor);
                }
                LiteralTableEntry::Value(value) => {
                    Self::visit_literal_expression(value, processor);
                }
            }
        }
    }

    fn visit_literal_expression(
        expression: &LiteralExpression,
        processor: &mut impl ImmutableProcessor,
    ) {
        processor.process_literal_expression(expression);
        match expression {
            LiteralExpression::Number(number) => Self::visit_number_expression(number, processor),
            LiteralExpression::String(string) => Self::visit_string_expression(string, processor),
            LiteralExpression::Table(table) => Self::visit_literal_table(table, processor),
            LiteralExpression::True(_)
            | LiteralExpression::False(_)
            | LiteralExpression::Nil(_) => {}
        }
    }

    fn visit_attributes(attributes: &Attributes, processor: &mut impl ImmutableProcessor) {
        processor.process_attributes(attributes);
        for attribute in attributes.iter_attributes() {
            match attribute {
                Attribute::Name(named) => {
                    Self::visit_identifier(named.get_identifier(), processor);
                }
                Attribute::Group(group) => {
                    for element in group.iter_attributes() {
                        Self::visit_identifier(element.name(), processor);

                        if let Some(arguments) = element.get_arguments() {
                            match arguments {
                                AttributeArguments::Tuple(arguments) => {
                                    for value in arguments.iter_values() {
                                        Self::visit_literal_expression(value, processor);
                                    }
                                }
                                AttributeArguments::String(string) => {
                                    Self::visit_string_expression(string, processor);
                                }
                                AttributeArguments::Table(table) => {
                                    Self::visit_literal_table(table, processor);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_function_signature<'a>(
        generic_parameters: Option<&GenericParameters>,
        parameters: impl Iterator<Item = &'a TypedIdentifier>,
        variadic_type: Option<&FunctionVariadicType>,
        return_type: Option<&FunctionReturnType>,
        processor: &mut impl ImmutableProcessor,
    ) {
        Self::visit_generic_parameters(generic_parameters, processor);
        for parameter in parameters {
            Self::visit_typed_identifier(parameter, processor);
        }
        if let Some(variadic_type) = variadic_type {
            Self::visit_function_variadic_type(variadic_type, processor);
        }
        if let Some(return_type) = return_type {
            Self::visit_function_return_type(return_type, processor);
        }
    }

    fn visit_function_variadic_type(
        variadic_type: &FunctionVariadicType,
        processor: &mut impl ImmutableProcessor,
    ) {
        match variadic_type {
            FunctionVariadicType::Type(r#type) => Self::visit_type(r#type, processor),
            FunctionVariadicType::GenericTypePack(generic) => {
                Self::visit_generic_type_pack(generic, processor)
            }
        }
    }

    fn visit_function_return_type(
        return_type: &FunctionReturnType,
        processor: &mut impl ImmutableProcessor,
    ) {
        match return_type {
            FunctionReturnType::Type(r#type) => Self::visit_type(r#type, processor),
            FunctionReturnType::TypePack(type_pack) => Self::visit_type_pack(type_pack, processor),
            FunctionReturnType::GenericTypePack(generic) => {
                Self::visit_generic_type_pack(generic, processor)
            }
            FunctionReturnType::VariadicTypePack(variadic) => {
                Self::visit_variadic_type_pack(variadic, processor)
            }
        }
    }

    fn visit_type_pack(type_pack: &TypePack, processor: &mut impl ImmutableProcessor) {
        for r#type in type_pack.iter() {
            Self::visit_type(r#type, processor);
        }
        if let Some(variadic) = type_pack.get_variadic_type() {
            Self::visit_variadic_argument_type(variadic, processor);
        }
    }

    fn visit_variadic_argument_type(
        variadic: &VariadicArgumentType,
        processor: &mut impl ImmutableProcessor,
    ) {
        match variadic {
            VariadicArgumentType::GenericTypePack(generic) => {
                Self::visit_generic_type_pack(generic, processor)
            }
            VariadicArgumentType::VariadicTypePack(variadic) => {
                Self::visit_variadic_type_pack(variadic, processor)
            }
        }
    }

    fn visit_variadic_type_pack(
        variadic: &VariadicTypePack,
        processor: &mut impl ImmutableProcessor,
    ) {
        Self::visit_type(variadic.get_type(), processor);
    }

    fn visit_generic_type_pack(generic: &GenericTypePack, processor: &mut impl ImmutableProcessor) {
        Self::visit_identifier(generic.get_name(), processor);
    }

    fn visit_generic_parameters(
        generic_parameters: Option<&GenericParameters>,
        processor: &mut impl ImmutableProcessor,
    ) {
        let Some(generic_parameters) = generic_parameters else {
            return;
        };
        for identifier in generic_parameters.iter_type_variable() {
            Self::visit_identifier(identifier, processor);
        }
        for generic in generic_parameters.iter_generic_type_pack() {
            Self::visit_generic_type_pack(generic, processor);
        }
    }

    fn visit_generic_parameters_with_defaults(
        generic_parameters: Option<&GenericParametersWithDefaults>,
        processor: &mut impl ImmutableProcessor,
    ) {
        let Some(generic_parameters) = generic_parameters else {
            return;
        };
        for parameter in generic_parameters.iter() {
            match parameter {
                GenericParameterRef::TypeVariable(identifier) => {
                    Self::visit_identifier(identifier, processor);
                }
                GenericParameterRef::TypeVariableWithDefault(parameter) => {
                    Self::visit_identifier(parameter.get_type_variable(), processor);
                    Self::visit_type(parameter.get_default_type(), processor);
                }
                GenericParameterRef::GenericTypePack(generic) => {
                    Self::visit_generic_type_pack(generic, processor);
                }
                GenericParameterRef::GenericTypePackWithDefault(parameter) => {
                    Self::visit_generic_type_pack(parameter.get_generic_type_pack(), processor);
                    match parameter.get_default_type() {
                        GenericTypePackDefault::TypePack(type_pack) => {
                            Self::visit_type_pack(type_pack, processor);
                        }
                        GenericTypePackDefault::VariadicTypePack(variadic) => {
                            Self::visit_variadic_type_pack(variadic, processor);
                        }
                        GenericTypePackDefault::GenericTypePack(generic) => {
                            Self::visit_generic_type_pack(generic, processor);
                        }
                    }
                }
            }
        }
    }

    fn visit_type(r#type: &Type, processor: &mut impl ImmutableProcessor) {
        processor.process_type(r#type);
        match r#type {
            Type::Name(type_name) => Self::visit_type_name(type_name, processor),
            Type::Field(field) => {
                Self::visit_identifier(field.get_namespace(), processor);
                Self::visit_type_name(field.get_type_name(), processor);
            }
            Type::True(_) | Type::False(_) | Type::Nil(_) | Type::String(_) => {}
            Type::Array(array) => Self::visit_type(array.get_element_type(), processor),
            Type::Table(table) => {
                for entry in table.iter_entries() {
                    match entry {
                        TableEntryType::Property(property) => {
                            Self::visit_identifier(property.get_identifier(), processor);
                            Self::visit_type(property.get_type(), processor);
                        }
                        TableEntryType::Literal(literal) => {
                            Self::visit_type(literal.get_type(), processor);
                        }
                        TableEntryType::Indexer(indexer) => {
                            Self::visit_type(indexer.get_key_type(), processor);
                            Self::visit_type(indexer.get_value_type(), processor);
                        }
                    }
                }
            }
            Type::TypeOf(expression_type) => {
                Self::visit_expression(expression_type.get_expression(), processor)
            }
            Type::Parenthese(parenthese) => {
                Self::visit_type(parenthese.get_inner_type(), processor)
            }
            Type::Function(function) => {
                Self::visit_generic_parameters(function.get_generic_parameters(), processor);
                for argument in function.iter_arguments() {
                    if let Some(name) = argument.get_name() {
                        Self::visit_identifier(name, processor);
                    }
                    Self::visit_type(argument.get_type(), processor);
                }
                Self::visit_function_return_type(function.get_return_type(), processor);
            }
            Type::Optional(optional) => Self::visit_type(optional.get_inner_type(), processor),
            Type::Intersection(intersection) => {
                for r#type in intersection.iter_types() {
                    Self::visit_type(r#type, processor);
                }
            }
            Type::Union(union) => {
                for r#type in union.iter_types() {
                    Self::visit_type(r#type, processor);
                }
            }
        }
    }

    fn visit_type_name(type_name: &TypeName, processor: &mut impl ImmutableProcessor) {
        processor.process_type_name(type_name);
        Self::visit_identifier(type_name.get_type_name(), processor);
        if let Some(parameters) = type_name.get_type_parameters() {
            for parameter in parameters.iter() {
                Self::visit_type_parameter(parameter, processor);
            }
        }
    }

    fn visit_type_parameter(parameter: &TypeParameter, processor: &mut impl ImmutableProcessor) {
        match parameter {
            TypeParameter::Type(r#type) => Self::visit_type(r#type, processor),
            TypeParameter::TypePack(type_pack) => Self::visit_type_pack(type_pack, processor),
            TypeParameter::VariadicTypePack(variadic) => {
                Self::visit_variadic_type_pack(variadic, processor)
            }
            TypeParameter::GenericTypePack(generic) => {
                Self::visit_generic_type_pack(generic, processor)
            }
        }
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
