use crate::nodes::*;
use crate::process::NodeProcessor;

use std::marker::PhantomData;

/// Adapts a [`NodeVisitor`] to the read-only/mutable child visitor used by the
/// authoritative child structure in [`crate::nodes::MutableChildren`].
struct MutableChildDispatch<'processor, T, V>
where
    T: NodeProcessor,
    V: NodeVisitor<T> + ?Sized,
{
    processor: &'processor mut T,
    _visitor: PhantomData<V>,
}

impl<T, V> ChildVisitorMut for MutableChildDispatch<'_, T, V>
where
    T: NodeProcessor,
    V: NodeVisitor<T> + ?Sized,
{
    fn block(&mut self, block: &mut Block) {
        V::visit_block(block, self.processor);
    }

    fn statement(&mut self, statement: &mut Statement) {
        V::visit_statement(statement, self.processor);
    }

    fn last_statement(&mut self, last_statement: &mut LastStatement) {
        V::visit_last_statement(last_statement, self.processor);
    }

    fn expression(&mut self, expression: &mut Expression) {
        V::visit_expression(expression, self.processor);
    }

    fn prefix(&mut self, prefix: &mut Prefix) {
        V::visit_prefix_expression(prefix, self.processor);
    }

    fn variable(&mut self, variable: &mut Variable) {
        V::visit_variable(variable, self.processor);
    }

    fn variable_expression(&mut self, identifier: &mut Identifier) {
        V::visit_identifier(identifier, self.processor);
    }

    fn attributes(&mut self, attributes: &mut Attributes) {
        V::visit_attributes(attributes, self.processor);
    }

    fn typed_identifier(&mut self, identifier: &mut TypedIdentifier) {
        MutableChildren::for_each_child_mut(identifier, self);
    }

    fn r#type(&mut self, r#type: &mut Type) {
        V::visit_type(r#type, self.processor);
    }

    fn type_name(&mut self, type_name: &mut TypeName) {
        V::visit_type_name(type_name, self.processor);
    }

    fn type_pack(&mut self, type_pack: &mut TypePack) {
        V::visit_type_pack(type_pack, self.processor);
    }

    fn generic_type_pack(&mut self, generic_type_pack: &mut GenericTypePack) {
        V::visit_generic_type_pack(generic_type_pack, self.processor);
    }

    fn variadic_type_pack(&mut self, variadic_type_pack: &mut VariadicTypePack) {
        V::visit_variadic_type_pack(variadic_type_pack, self.processor);
    }

    fn variadic_argument_type(&mut self, argument: &mut VariadicArgumentType) {
        V::visit_variadic_argument_type(argument, self.processor);
    }

    fn function_variadic_type(&mut self, variadic: &mut FunctionVariadicType) {
        V::visit_function_variadic_type(variadic, self.processor);
    }

    fn function_return_type(&mut self, r#type: &mut FunctionReturnType) {
        V::visit_function_return_type(r#type, self.processor);
    }

    fn generic_parameters_with_defaults(&mut self, parameters: &mut GenericParametersWithDefaults) {
        MutableChildren::for_each_child_mut(parameters, self);
    }

    fn table(&mut self, table: &mut TableExpression) {
        V::visit_table(table, self.processor);
    }

    fn arguments(&mut self, arguments: &mut Arguments) {
        V::visit_arguments(arguments, self.processor);
    }

    fn string_argument(&mut self, string: &mut StringExpression) {
        V::visit_string_expression(string, self.processor);
    }

    fn literal_expression(&mut self, expression: &mut LiteralExpression) {
        V::visit_literal_expression(expression, self.processor);
    }

    fn literal_argument(&mut self, expression: &mut LiteralExpression) {
        MutableChildren::for_each_child_mut(expression, self);
    }

    fn literal_table(&mut self, table: &mut LiteralTable) {
        V::visit_literal_table(table, self.processor);
    }

    fn string_type(&mut self, string: &mut StringType) {
        V::visit_string_type(string, self.processor);
    }

    fn number_expression(&mut self, number: &mut NumberExpression) {
        V::visit_number_expression(number, self.processor);
    }

    fn string_expression(&mut self, string: &mut StringExpression) {
        V::visit_string_expression(string, self.processor);
    }
}

/// A trait that defines method that iterates on nodes and process them using a NodeProcessor.
pub trait NodeVisitor<T: NodeProcessor> {
    /// Recurses into the children of `node` through the single authoritative
    /// child definition, dispatching each child to the matching `visit_*`
    /// method.
    fn visit_children_mut<C>(node: &mut C, processor: &mut T)
    where
        C: MutableChildren,
    {
        let mut dispatch = MutableChildDispatch::<T, Self> {
            processor,
            _visitor: PhantomData,
        };
        node.for_each_child_mut(&mut dispatch);
    }

    fn visit_block(block: &mut Block, processor: &mut T) {
        processor.process_block(block);
        Self::visit_children_mut(block, processor);
    }

    fn visit_statement(statement: &mut Statement, processor: &mut T) {
        processor.process_statement(statement);

        match statement {
            Statement::Assign(statement) => Self::visit_assign_statement(statement, processor),
            Statement::Do(statement) => Self::visit_do_statement(statement, processor),
            Statement::Call(statement) => Self::visit_function_call(statement, processor),
            Statement::CompoundAssign(statement) => {
                Self::visit_compound_assign(statement, processor)
            }
            Statement::Function(statement) => Self::visit_function_statement(statement, processor),
            Statement::GenericFor(statement) => Self::visit_generic_for(statement, processor),
            Statement::Goto(statement) => Self::visit_goto_statement(statement, processor),
            Statement::If(statement) => Self::visit_if_statement(statement, processor),
            Statement::Label(statement) => Self::visit_label_statement(statement, processor),
            Statement::LocalAssign(statement) => Self::visit_local_assign(statement, processor),
            Statement::LocalFunction(statement) => Self::visit_local_function(statement, processor),
            Statement::NumericFor(statement) => Self::visit_numeric_for(statement, processor),
            Statement::Repeat(statement) => Self::visit_repeat_statement(statement, processor),
            Statement::While(statement) => Self::visit_while_statement(statement, processor),
            Statement::TypeDeclaration(statement) => {
                Self::visit_type_declaration(statement, processor)
            }
            Statement::TypeFunction(statement) => Self::visit_type_function(statement, processor),
        };
    }

    fn visit_last_statement(last_statement: &mut LastStatement, processor: &mut T) {
        processor.process_last_statement(last_statement);
        Self::visit_children_mut(last_statement, processor);
    }

    fn visit_expression(expression: &mut Expression, processor: &mut T) {
        processor.process_expression(expression);

        match expression {
            Expression::Binary(expression) => {
                Self::visit_binary_expression(expression, processor);
            }
            Expression::Call(expression) => Self::visit_function_call(expression, processor),
            Expression::Field(field) => Self::visit_field_expression(field, processor),
            Expression::Function(function) => Self::visit_function_expression(function, processor),
            Expression::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Expression::If(if_expression) => Self::visit_if_expression(if_expression, processor),
            Expression::Index(index) => Self::visit_index_expression(index, processor),
            Expression::Number(number) => Self::visit_number_expression(number, processor),
            Expression::Parenthese(expression) => {
                Self::visit_parenthese_expression(expression, processor);
            }
            Expression::String(string) => {
                Self::visit_string_expression(string, processor);
            }
            Expression::InterpolatedString(interpolated_string) => {
                Self::visit_interpolated_string_expression(interpolated_string, processor);
            }
            Expression::Table(table) => Self::visit_table(table, processor),
            Expression::Unary(unary) => {
                Self::visit_unary_expression(unary, processor);
            }
            Expression::TypeCast(type_cast) => {
                Self::visit_type_cast_expression(type_cast, processor);
            }
            Expression::TypeInstantiation(type_instantiation) => {
                Self::visit_type_instantiation(type_instantiation, processor);
            }
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => {}
        }
    }

    fn visit_binary_expression(binary: &mut BinaryExpression, processor: &mut T) {
        processor.process_binary_expression(binary);
        Self::visit_children_mut(binary, processor);
    }

    fn visit_number_expression(number: &mut NumberExpression, processor: &mut T) {
        processor.process_number_expression(number);
    }

    fn visit_parenthese_expression(parenthese: &mut ParentheseExpression, processor: &mut T) {
        processor.process_parenthese_expression(parenthese);
        Self::visit_children_mut(parenthese, processor);
    }

    fn visit_string_expression(string: &mut StringExpression, processor: &mut T) {
        processor.process_string_expression(string);
    }

    fn visit_interpolated_string_expression(
        interpolated_string: &mut InterpolatedStringExpression,
        processor: &mut T,
    ) {
        processor.process_interpolated_string_expression(interpolated_string);
        Self::visit_children_mut(interpolated_string, processor);
    }

    fn visit_unary_expression(unary: &mut UnaryExpression, processor: &mut T) {
        processor.process_unary_expression(unary);
        Self::visit_children_mut(unary, processor);
    }

    fn visit_type_cast_expression(type_cast: &mut TypeCastExpression, processor: &mut T) {
        processor.process_type_cast_expression(type_cast);
        Self::visit_children_mut(type_cast, processor);
    }

    fn visit_type_instantiation(
        type_instantiation: &mut TypeInstantiationExpression,
        processor: &mut T,
    ) {
        processor.process_type_instantiation(type_instantiation);
        Self::visit_children_mut(type_instantiation, processor);
    }

    fn visit_function_expression(function: &mut FunctionExpression, processor: &mut T) {
        processor.process_function_expression(function);
        processor.process_scope(function.mutate_block(), None);
        Self::visit_children_mut(function, processor);
    }

    fn visit_assign_statement(statement: &mut AssignStatement, processor: &mut T) {
        processor.process_assign_statement(statement);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_do_statement(statement: &mut DoStatement, processor: &mut T) {
        processor.process_do_statement(statement);
        processor.process_scope(statement.mutate_block(), None);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_compound_assign(statement: &mut CompoundAssignStatement, processor: &mut T) {
        processor.process_compound_assign_statement(statement);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_function_statement(statement: &mut FunctionStatement, processor: &mut T) {
        processor.process_function_statement(statement);
        processor.process_scope(statement.mutate_block(), None);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_generic_for(statement: &mut GenericForStatement, processor: &mut T) {
        processor.process_generic_for_statement(statement);
        processor.process_scope(statement.mutate_block(), None);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_goto_statement(statement: &mut GotoStatement, processor: &mut T) {
        processor.process_goto_statement(statement);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_label_statement(statement: &mut LabelStatement, processor: &mut T) {
        processor.process_label_statement(statement);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_if_statement(statement: &mut IfStatement, processor: &mut T) {
        processor.process_if_statement(statement);

        // `process_scope` has no implementors; it is kept for API compatibility
        // and called for each branch block before the children are visited.
        for branch in statement.mutate_branches() {
            processor.process_scope(branch.mutate_block(), None);
        }
        if let Some(block) = statement.mutate_else_block() {
            processor.process_scope(block, None);
        }

        Self::visit_children_mut(statement, processor);
    }

    fn visit_local_assign(statement: &mut VariableAssignment, processor: &mut T) {
        processor.process_local_assign_statement(statement);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_local_function(statement: &mut FunctionAssignment, processor: &mut T) {
        processor.process_local_function_statement(statement);
        processor.process_scope(statement.mutate_block(), None);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_function_variadic_type(variadic_type: &mut FunctionVariadicType, processor: &mut T) {
        Self::visit_children_mut(variadic_type, processor);
    }

    fn visit_generic_type_pack(generic: &mut GenericTypePack, processor: &mut T) {
        processor.process_generic_type_pack(generic);
        Self::visit_children_mut(generic, processor);
    }

    fn visit_numeric_for(statement: &mut NumericForStatement, processor: &mut T) {
        processor.process_numeric_for_statement(statement);
        processor.process_scope(statement.mutate_block(), None);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_repeat_statement(statement: &mut RepeatStatement, processor: &mut T) {
        processor.process_repeat_statement(statement);

        let (block, condition) = statement.mutate_block_and_condition();
        processor.process_scope(block, Some(condition));

        Self::visit_children_mut(statement, processor);
    }

    fn visit_while_statement(statement: &mut WhileStatement, processor: &mut T) {
        processor.process_while_statement(statement);
        processor.process_scope(statement.mutate_block(), None);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_type_declaration(statement: &mut TypeDeclarationStatement, processor: &mut T) {
        processor.process_type_declaration(statement);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_type_function(statement: &mut TypeFunctionStatement, processor: &mut T) {
        processor.process_type_function(statement);
        processor.process_scope(statement.mutate_block(), None);
        Self::visit_children_mut(statement, processor);
    }

    fn visit_variable(variable: &mut Variable, processor: &mut T) {
        processor.process_variable(variable);

        match variable {
            Variable::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Variable::Field(field) => Self::visit_field_expression(field, processor),
            Variable::Index(index) => Self::visit_index_expression(index, processor),
        }
    }

    fn visit_identifier(identifier: &mut Identifier, processor: &mut T) {
        processor.process_variable_expression(identifier);
    }

    fn visit_if_expression(if_expression: &mut IfExpression, processor: &mut T) {
        processor.process_if_expression(if_expression);
        Self::visit_children_mut(if_expression, processor);
    }

    fn visit_field_expression(field: &mut FieldExpression, processor: &mut T) {
        processor.process_field_expression(field);
        Self::visit_children_mut(field, processor);
    }

    fn visit_index_expression(index: &mut IndexExpression, processor: &mut T) {
        processor.process_index_expression(index);
        Self::visit_children_mut(index, processor);
    }

    fn visit_function_call(call: &mut FunctionCall, processor: &mut T) {
        processor.process_function_call(call);
        Self::visit_children_mut(call, processor);
    }

    fn visit_arguments(arguments: &mut Arguments, processor: &mut T) {
        Self::visit_children_mut(arguments, processor);
    }

    fn visit_table(table: &mut TableExpression, processor: &mut T) {
        processor.process_table_expression(table);
        Self::visit_children_mut(table, processor);
    }

    fn visit_prefix_expression(prefix: &mut Prefix, processor: &mut T) {
        processor.process_prefix_expression(prefix);

        match prefix {
            Prefix::Call(call) => Self::visit_function_call(call, processor),
            Prefix::Field(field) => Self::visit_field_expression(field, processor),
            Prefix::Identifier(identifier) => Self::visit_identifier(identifier, processor),
            Prefix::Index(index) => Self::visit_index_expression(index, processor),
            Prefix::Parenthese(expression) => {
                Self::visit_parenthese_expression(expression, processor)
            }
            Prefix::TypeInstantiation(type_instantiation) => {
                Self::visit_type_instantiation(type_instantiation, processor);
            }
        };
    }

    fn visit_attributes(attributes: &mut Attributes, processor: &mut T) {
        processor.process_attributes(attributes);
        Self::visit_children_mut(attributes, processor);
    }

    fn visit_literal_table(table: &mut LiteralTable, processor: &mut T) {
        processor.process_literal_table(table);
        Self::visit_children_mut(table, processor);
    }

    fn visit_literal_expression(expression: &mut LiteralExpression, processor: &mut T) {
        processor.process_literal_expression(expression);
        Self::visit_children_mut(expression, processor);
    }

    fn visit_type(r#type: &mut Type, processor: &mut T) {
        processor.process_type(r#type);

        match r#type {
            Type::Name(type_name) => Self::visit_type_name(type_name, processor),
            Type::Field(type_field) => Self::visit_type_field(type_field, processor),
            Type::Array(array) => Self::visit_array_type(array, processor),
            Type::Table(table) => Self::visit_table_type(table, processor),
            Type::TypeOf(expression_type) => {
                Self::visit_expression_type(expression_type, processor)
            }
            Type::Parenthese(parenthese) => Self::visit_parenthese_type(parenthese, processor),
            Type::Function(function) => Self::visit_function_type(function, processor),
            Type::Optional(optional) => Self::visit_optional_type(optional, processor),
            Type::Intersection(intersection) => {
                Self::visit_intersection_type(intersection, processor)
            }
            Type::Union(union) => Self::visit_union_type(union, processor),
            Type::String(string) => Self::visit_string_type(string, processor),
            Type::True(_) | Type::False(_) | Type::Nil(_) => {}
        }
    }

    fn visit_type_name(type_name: &mut TypeName, processor: &mut T) {
        processor.process_type_name(type_name);
        Self::visit_children_mut(type_name, processor);
    }

    fn visit_type_field(type_field: &mut TypeField, processor: &mut T) {
        processor.process_type_field(type_field);
        Self::visit_children_mut(type_field, processor);
    }

    fn visit_array_type(array: &mut ArrayType, processor: &mut T) {
        processor.process_array_type(array);
        Self::visit_children_mut(array, processor);
    }

    fn visit_table_type(table: &mut TableType, processor: &mut T) {
        processor.process_table_type(table);
        Self::visit_children_mut(table, processor);
    }

    fn visit_expression_type(expression_type: &mut ExpressionType, processor: &mut T) {
        processor.process_expression_type(expression_type);
        Self::visit_children_mut(expression_type, processor);
    }

    fn visit_parenthese_type(parenthese: &mut ParentheseType, processor: &mut T) {
        processor.process_parenthese_type(parenthese);
        Self::visit_children_mut(parenthese, processor);
    }

    fn visit_function_type(function: &mut FunctionType, processor: &mut T) {
        processor.process_function_type(function);
        Self::visit_children_mut(function, processor);
    }

    fn visit_optional_type(optional: &mut OptionalType, processor: &mut T) {
        processor.process_optional_type(optional);
        Self::visit_children_mut(optional, processor);
    }

    fn visit_intersection_type(intersection: &mut IntersectionType, processor: &mut T) {
        processor.process_intersection_type(intersection);
        Self::visit_children_mut(intersection, processor);
    }

    fn visit_union_type(union: &mut UnionType, processor: &mut T) {
        processor.process_union_type(union);
        Self::visit_children_mut(union, processor);
    }

    fn visit_string_type(string: &mut StringType, processor: &mut T) {
        processor.process_string_type(string);
    }

    fn visit_type_pack(type_pack: &mut TypePack, processor: &mut T) {
        processor.process_type_pack(type_pack);
        Self::visit_children_mut(type_pack, processor);
    }

    fn visit_variadic_type_pack(variadic_type_pack: &mut VariadicTypePack, processor: &mut T) {
        processor.process_variadic_type_pack(variadic_type_pack);
        Self::visit_children_mut(variadic_type_pack, processor);
    }

    fn visit_variadic_argument_type(variadic_type: &mut VariadicArgumentType, processor: &mut T) {
        Self::visit_children_mut(variadic_type, processor);
    }

    fn visit_function_return_type(
        function_return_type: &mut FunctionReturnType,
        processor: &mut T,
    ) {
        Self::visit_children_mut(function_return_type, processor);
    }
}

/// The default node visitor.
pub struct DefaultVisitor<T> {
    _phantom: PhantomData<T>,
}

impl<T: NodeProcessor> NodeVisitor<T> for DefaultVisitor<T> {}

#[cfg(test)]
mod test {
    use super::*;
    use crate::process::NodeCounter;

    #[test]
    fn visit_do_statement() {
        let mut counter = NodeCounter::new();
        let mut block = Block::default().with_statement(DoStatement::default());

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 2);
        assert_eq!(counter.do_count, 1);
    }

    #[test]
    fn visit_numeric_for_statement() {
        let mut counter = NodeCounter::new();
        let mut block = Block::default().with_statement(NumericForStatement::new(
            "i".to_owned(),
            Expression::from(true),
            Expression::from(true),
            None,
            Block::default(),
        ));

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 2);
        assert_eq!(counter.expression_count, 2);
        assert_eq!(counter.numeric_for_count, 1);
    }

    #[test]
    fn visit_generic_for_statement() {
        let mut counter = NodeCounter::new();
        let mut block = Block::default().with_statement(GenericForStatement::new(
            vec!["k".into()],
            vec![Expression::from(true)],
            Block::default(),
        ));

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 2);
        assert_eq!(counter.expression_count, 1);
        assert_eq!(counter.generic_for_count, 1);
    }

    #[test]
    fn visit_repeat_statement() {
        let mut counter = NodeCounter::new();
        let mut block =
            Block::default().with_statement(RepeatStatement::new(Block::default(), true));

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 2);
        assert_eq!(counter.expression_count, 1);
        assert_eq!(counter.repeat_count, 1);
    }

    #[test]
    fn visit_while_statement() {
        let mut counter = NodeCounter::new();
        let mut block =
            Block::default().with_statement(WhileStatement::new(Block::default(), true));

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 2);
        assert_eq!(counter.expression_count, 1);
        assert_eq!(counter.while_count, 1);
    }

    #[test]
    fn visit_if_statement() {
        let mut counter = NodeCounter::new();
        let mut block =
            Block::default().with_statement(IfStatement::create(true, Block::default()));

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 2);
        assert_eq!(counter.expression_count, 1);
        assert_eq!(counter.if_count, 1);
    }

    #[test]
    fn visit_if_statement_with_else() {
        let mut counter = NodeCounter::new();
        let if_statement =
            IfStatement::create(true, Block::default()).with_else_block(Block::default());

        let mut block = Block::default().with_statement(if_statement);

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 3);
        assert_eq!(counter.expression_count, 1);
        assert_eq!(counter.if_count, 1);
    }

    #[test]
    fn visit_if_statement_with_elseif_and_else() {
        let mut counter = NodeCounter::new();
        let if_statement = IfStatement::create(true, Block::default())
            .with_new_branch(false, Block::default())
            .with_else_block(Block::default());

        let mut block = Block::default().with_statement(if_statement);

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.block_count, 4);
        assert_eq!(counter.expression_count, 2);
        assert_eq!(counter.if_count, 1);
    }

    #[test]
    fn visit_compound_assign_statement() {
        let mut counter = NodeCounter::new();
        let statement =
            CompoundAssignStatement::new(CompoundOperator::Plus, Variable::new("var"), 1_f64);

        let mut block = statement.into();

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.compound_assign, 1);
        assert_eq!(counter.expression_count, 1);
        assert_eq!(counter.variable_count, 1);
    }

    #[test]
    fn visit_interpolated_string() {
        let mut counter = NodeCounter::new();
        let statement = VariableAssignment::from_variable("value")
            .with_value(InterpolatedStringExpression::empty().with_segment(Expression::from(true)));

        let mut block = statement.into();

        DefaultVisitor::visit_block(&mut block, &mut counter);

        assert_eq!(counter.interpolated_string_count, 1);
        assert_eq!(counter.expression_count, 2);
    }
}
