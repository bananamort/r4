//! The single authoritative definition of AST child structure.
//!
//! Every node that the three process visitors recurse into lists its children
//! here exactly once, through [`ReadOnlyChildren::for_each_child`] and
//! [`MutableChildren::for_each_child_mut`]. The visitors only decide which
//! processor hook fires for each child; they never re-state the child lists.
//!
//! The child callbacks are grouped by the *container kind* stored on a node
//! (a statement, an expression, a type, an identifier, a type pack, ...), not
//! by the enum variant of the child. The mutable and read-only traits expose the
//! same callbacks so a change to a node's shape is made in one place.

use crate::nodes::*;

/// A read-only receiver for the children enumerated by
/// [`ReadOnlyChildren::for_each_child`].
///
/// Every method has an empty default body so a visitor only implements the
/// child kinds it cares about.
pub trait ChildVisitor {
    fn block(&mut self, _block: &Block) {}
    fn statement(&mut self, _statement: &Statement) {}
    fn last_statement(&mut self, _statement: &LastStatement) {}
    fn expression(&mut self, _expression: &Expression) {}
    fn prefix(&mut self, _prefix: &Prefix) {}
    fn variable(&mut self, _variable: &Variable) {}
    /// An identifier that is *not* delivered as a variable expression.
    fn identifier(&mut self, _identifier: &Identifier) {}
    /// An identifier that stands for a variable expression.
    fn variable_expression(&mut self, _identifier: &Identifier) {}
    fn attributes(&mut self, _attributes: &Attributes) {}
    fn typed_identifier(&mut self, _identifier: &TypedIdentifier) {}
    fn r#type(&mut self, _type_value: &Type) {}
    fn type_name(&mut self, _type_name: &TypeName) {}
    fn type_pack(&mut self, _type_pack: &TypePack) {}
    fn generic_type_pack(&mut self, _generic_type_pack: &GenericTypePack) {}
    fn variadic_type_pack(&mut self, _variadic_type_pack: &VariadicTypePack) {}
    fn variadic_argument_type(&mut self, _argument: &VariadicArgumentType) {}
    fn function_variadic_type(&mut self, _variadic: &FunctionVariadicType) {}
    fn function_return_type(&mut self, _type_value: &FunctionReturnType) {}
    fn generic_parameters(&mut self, _parameters: &GenericParameters) {}
    fn generic_parameters_with_defaults(&mut self, _parameters: &GenericParametersWithDefaults) {}
    fn table(&mut self, _table: &TableExpression) {}
    fn arguments(&mut self, _arguments: &Arguments) {}
    /// A string argument in a call position (`f "text"`); the immutable walk
    /// does not surface these.
    fn string_argument(&mut self, _string: &StringExpression) {}
    fn literal_expression(&mut self, _expression: &LiteralExpression) {}
    /// A literal expression in a position where the mutable visitor does not
    /// surface `process_literal_expression`, such as an attribute argument.
    fn literal_argument(&mut self, _expression: &LiteralExpression) {}
    fn literal_table(&mut self, _table: &LiteralTable) {}
    fn string_type(&mut self, _string: &StringType) {}
    fn number_expression(&mut self, _number: &NumberExpression) {}
    fn string_expression(&mut self, _string: &StringExpression) {}
}

/// The mutable counterpart of [`ChildVisitor`].
pub trait ChildVisitorMut {
    fn block(&mut self, _block: &mut Block) {}
    fn statement(&mut self, _statement: &mut Statement) {}
    fn last_statement(&mut self, _statement: &mut LastStatement) {}
    fn expression(&mut self, _expression: &mut Expression) {}
    fn prefix(&mut self, _prefix: &mut Prefix) {}
    fn variable(&mut self, _variable: &mut Variable) {}
    fn identifier(&mut self, _identifier: &mut Identifier) {}
    fn variable_expression(&mut self, _identifier: &mut Identifier) {}
    fn attributes(&mut self, _attributes: &mut Attributes) {}
    fn typed_identifier(&mut self, _identifier: &mut TypedIdentifier) {}
    fn r#type(&mut self, _type_value: &mut Type) {}
    fn type_name(&mut self, _type_name: &mut TypeName) {}
    fn type_pack(&mut self, _type_pack: &mut TypePack) {}
    fn generic_type_pack(&mut self, _generic_type_pack: &mut GenericTypePack) {}
    fn variadic_type_pack(&mut self, _variadic_type_pack: &mut VariadicTypePack) {}
    fn variadic_argument_type(&mut self, _argument: &mut VariadicArgumentType) {}
    fn function_variadic_type(&mut self, _variadic: &mut FunctionVariadicType) {}
    fn function_return_type(&mut self, _type_value: &mut FunctionReturnType) {}
    fn generic_parameters(&mut self, _parameters: &mut GenericParameters) {}
    fn generic_parameters_with_defaults(
        &mut self,
        _parameters: &mut GenericParametersWithDefaults,
    ) {
    }
    fn table(&mut self, _table: &mut TableExpression) {}
    fn arguments(&mut self, _arguments: &mut Arguments) {}
    fn string_argument(&mut self, _string: &mut StringExpression) {}
    fn literal_expression(&mut self, _expression: &mut LiteralExpression) {}
    fn literal_argument(&mut self, _expression: &mut LiteralExpression) {}
    fn literal_table(&mut self, _table: &mut LiteralTable) {}
    fn string_type(&mut self, _string: &mut StringType) {}
    fn number_expression(&mut self, _number: &mut NumberExpression) {}
    fn string_expression(&mut self, _string: &mut StringExpression) {}
}

/// A node whose children can be enumerated read-only.
pub trait ReadOnlyChildren {
    /// Calls `visitor` once for each child container, in the order the mutable
    /// visitor visits them today.
    fn for_each_child(&self, visitor: &mut impl ChildVisitor);
}

/// A node whose children can be enumerated mutably.
pub trait MutableChildren {
    /// Calls `visitor` once for each child container, in the order the mutable
    /// visitor visits them today.
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut);
}

impl<T: ReadOnlyChildren + ?Sized> ReadOnlyChildren for Box<T> {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        (**self).for_each_child(visitor);
    }
}

impl<T: MutableChildren + ?Sized> MutableChildren for Box<T> {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        (**self).for_each_child_mut(visitor);
    }
}

impl ReadOnlyChildren for Block {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for statement in self.iter_statements() {
            visitor.statement(statement);
        }
        if let Some(last_statement) = self.get_last_statement() {
            visitor.last_statement(last_statement);
        }
    }
}

impl MutableChildren for Block {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for statement in self.iter_mut_statements() {
            visitor.statement(statement);
        }
        if let Some(last_statement) = self.mutate_last_statement() {
            visitor.last_statement(last_statement);
        }
    }
}

impl ReadOnlyChildren for LastStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        if let LastStatement::Return(statement) = self {
            for expression in statement.iter_expressions() {
                visitor.expression(expression);
            }
        }
    }
}

impl MutableChildren for LastStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        if let LastStatement::Return(statement) = self {
            for expression in statement.iter_mut_expressions() {
                visitor.expression(expression);
            }
        }
    }
}

impl ReadOnlyChildren for Statement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            Statement::Assign(statement) => statement.for_each_child(visitor),
            Statement::Do(statement) => statement.for_each_child(visitor),
            Statement::Call(call) => call.for_each_child(visitor),
            Statement::CompoundAssign(statement) => statement.for_each_child(visitor),
            Statement::Function(statement) => statement.for_each_child(visitor),
            Statement::Goto(statement) => statement.for_each_child(visitor),
            Statement::GenericFor(statement) => statement.for_each_child(visitor),
            Statement::If(statement) => statement.for_each_child(visitor),
            Statement::Label(statement) => statement.for_each_child(visitor),
            Statement::LocalAssign(statement) => statement.for_each_child(visitor),
            Statement::LocalFunction(statement) => statement.for_each_child(visitor),
            Statement::NumericFor(statement) => statement.for_each_child(visitor),
            Statement::Repeat(statement) => statement.for_each_child(visitor),
            Statement::While(statement) => statement.for_each_child(visitor),
            Statement::TypeDeclaration(statement) => statement.for_each_child(visitor),
            Statement::TypeFunction(statement) => statement.for_each_child(visitor),
        }
    }
}

impl MutableChildren for Statement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            Statement::Assign(statement) => statement.for_each_child_mut(visitor),
            Statement::Do(statement) => statement.for_each_child_mut(visitor),
            Statement::Call(call) => call.for_each_child_mut(visitor),
            Statement::CompoundAssign(statement) => statement.for_each_child_mut(visitor),
            Statement::Function(statement) => statement.for_each_child_mut(visitor),
            Statement::Goto(statement) => statement.for_each_child_mut(visitor),
            Statement::GenericFor(statement) => statement.for_each_child_mut(visitor),
            Statement::If(statement) => statement.for_each_child_mut(visitor),
            Statement::Label(statement) => statement.for_each_child_mut(visitor),
            Statement::LocalAssign(statement) => statement.for_each_child_mut(visitor),
            Statement::LocalFunction(statement) => statement.for_each_child_mut(visitor),
            Statement::NumericFor(statement) => statement.for_each_child_mut(visitor),
            Statement::Repeat(statement) => statement.for_each_child_mut(visitor),
            Statement::While(statement) => statement.for_each_child_mut(visitor),
            Statement::TypeDeclaration(statement) => statement.for_each_child_mut(visitor),
            Statement::TypeFunction(statement) => statement.for_each_child_mut(visitor),
        }
    }
}

impl ReadOnlyChildren for AssignStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for variable in self.iter_variables() {
            visitor.variable(variable);
        }
        for value in self.iter_values() {
            visitor.expression(value);
        }
    }
}

impl MutableChildren for AssignStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for variable in self.iter_mut_variables() {
            visitor.variable(variable);
        }
        for value in self.iter_mut_values() {
            visitor.expression(value);
        }
    }
}

impl ReadOnlyChildren for DoStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.block(self.get_block());
    }
}

impl MutableChildren for DoStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.block(self.mutate_block());
    }
}

impl ReadOnlyChildren for CompoundAssignStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.variable(self.get_variable());
        visitor.expression(self.get_value());
    }
}

impl MutableChildren for CompoundAssignStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.variable(self.mutate_variable());
        visitor.expression(self.mutate_value());
    }
}

impl ReadOnlyChildren for FunctionName {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.variable_expression(self.get_name());
        for field in self.get_field_names() {
            visitor.identifier(field);
        }
        if let Some(method) = self.get_method() {
            visitor.identifier(method);
        }
    }
}

impl MutableChildren for FunctionName {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.variable_expression(self.mutate_identifier());
        for field in self.mutate_field_names() {
            visitor.identifier(field);
        }
        if let Some(method) = self.mutate_method() {
            visitor.identifier(method);
        }
    }
}

impl ReadOnlyChildren for FunctionStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.attributes(self.attributes());
        self.get_name().for_each_child(visitor);
        if let Some(generic_parameters) = self.get_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.get_block());
        for parameter in self.iter_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.get_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.get_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl MutableChildren for FunctionStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.attributes(self.mutate_attributes());
        self.mutate_function_name().for_each_child_mut(visitor);
        if let Some(generic_parameters) = self.mutate_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.mutate_block());
        for parameter in self.iter_mut_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.mutate_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.mutate_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl ReadOnlyChildren for GenericForStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for expression in self.iter_expressions() {
            visitor.expression(expression);
        }
        visitor.block(self.get_block());
        for identifier in self.iter_identifiers() {
            visitor.typed_identifier(identifier);
        }
    }
}

impl MutableChildren for GenericForStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for expression in self.iter_mut_expressions() {
            visitor.expression(expression);
        }
        visitor.block(self.mutate_block());
        for identifier in self.iter_mut_identifiers() {
            visitor.typed_identifier(identifier);
        }
    }
}

impl ReadOnlyChildren for GotoStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_label());
    }
}

impl MutableChildren for GotoStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_label());
    }
}

impl ReadOnlyChildren for LabelStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_label());
    }
}

impl MutableChildren for LabelStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_label());
    }
}

impl ReadOnlyChildren for IfBranch {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_condition());
        visitor.block(self.get_block());
    }
}

impl MutableChildren for IfBranch {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_condition());
        visitor.block(self.mutate_block());
    }
}

impl ReadOnlyChildren for IfStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for branch in self.iter_branches() {
            branch.for_each_child(visitor);
        }
        if let Some(else_block) = self.get_else_block() {
            visitor.block(else_block);
        }
    }
}

impl MutableChildren for IfStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for branch in self.mutate_branches() {
            branch.for_each_child_mut(visitor);
        }
        if let Some(else_block) = self.mutate_else_block() {
            visitor.block(else_block);
        }
    }
}

impl ReadOnlyChildren for VariableAssignment {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for value in self.iter_values() {
            visitor.expression(value);
        }
        for variable in self.iter_variables() {
            visitor.typed_identifier(variable);
        }
    }
}

impl MutableChildren for VariableAssignment {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for value in self.iter_mut_values() {
            visitor.expression(value);
        }
        for variable in self.iter_mut_variables() {
            visitor.typed_identifier(variable);
        }
    }
}

impl ReadOnlyChildren for FunctionAssignment {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.attributes(self.attributes());
        visitor.identifier(self.get_identifier());
        if let Some(generic_parameters) = self.get_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.get_block());
        for parameter in self.iter_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.get_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.get_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl MutableChildren for FunctionAssignment {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.attributes(self.mutate_attributes());
        visitor.identifier(self.mutate_identifier());
        if let Some(generic_parameters) = self.mutate_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.mutate_block());
        for parameter in self.iter_mut_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.mutate_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.mutate_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl ReadOnlyChildren for NumericForStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_start());
        visitor.expression(self.get_end());
        if let Some(step) = self.get_step() {
            visitor.expression(step);
        }
        visitor.block(self.get_block());
        visitor.typed_identifier(self.get_identifier());
    }
}

impl MutableChildren for NumericForStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_start());
        visitor.expression(self.mutate_end());
        if let Some(step) = self.mutate_step() {
            visitor.expression(step);
        }
        visitor.block(self.mutate_block());
        visitor.typed_identifier(self.mutate_identifier());
    }
}

impl ReadOnlyChildren for RepeatStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_condition());
        visitor.block(self.get_block());
    }
}

impl MutableChildren for RepeatStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_condition());
        visitor.block(self.mutate_block());
    }
}

impl ReadOnlyChildren for WhileStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_condition());
        visitor.block(self.get_block());
    }
}

impl MutableChildren for WhileStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_condition());
        visitor.block(self.mutate_block());
    }
}

impl ReadOnlyChildren for TypeDeclarationStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_name());
        if let Some(generic_parameters) = self.get_generic_parameters() {
            visitor.generic_parameters_with_defaults(generic_parameters);
        }
        visitor.r#type(self.get_type());
    }
}

impl MutableChildren for TypeDeclarationStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_name());
        if let Some(generic_parameters) = self.mutate_generic_parameters() {
            visitor.generic_parameters_with_defaults(generic_parameters);
        }
        visitor.r#type(self.mutate_type());
    }
}

impl ReadOnlyChildren for TypeFunctionStatement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        if let Some(generic_parameters) = self.get_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.get_block());
        for parameter in self.iter_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.get_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.get_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl MutableChildren for TypeFunctionStatement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        if let Some(generic_parameters) = self.mutate_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.mutate_block());
        for parameter in self.iter_mut_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.mutate_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.mutate_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl ReadOnlyChildren for Expression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            Expression::Binary(expression) => expression.for_each_child(visitor),
            Expression::Call(call) => call.for_each_child(visitor),
            Expression::Field(field) => field.for_each_child(visitor),
            Expression::Function(function) => function.for_each_child(visitor),
            Expression::Identifier(identifier) => visitor.variable_expression(identifier),
            Expression::If(if_expression) => if_expression.for_each_child(visitor),
            Expression::Index(index) => index.for_each_child(visitor),
            Expression::InterpolatedString(string) => string.for_each_child(visitor),
            Expression::Number(number) => number.for_each_child(visitor),
            Expression::Parenthese(parenthese) => parenthese.for_each_child(visitor),
            Expression::String(string) => string.for_each_child(visitor),
            Expression::Table(table) => table.for_each_child(visitor),
            Expression::Unary(unary) => unary.for_each_child(visitor),
            Expression::TypeCast(type_cast) => type_cast.for_each_child(visitor),
            Expression::TypeInstantiation(instantiation) => instantiation.for_each_child(visitor),
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => {}
        }
    }
}

impl MutableChildren for Expression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            Expression::Binary(expression) => expression.for_each_child_mut(visitor),
            Expression::Call(call) => call.for_each_child_mut(visitor),
            Expression::Field(field) => field.for_each_child_mut(visitor),
            Expression::Function(function) => function.for_each_child_mut(visitor),
            Expression::Identifier(identifier) => visitor.variable_expression(identifier),
            Expression::If(if_expression) => if_expression.for_each_child_mut(visitor),
            Expression::Index(index) => index.for_each_child_mut(visitor),
            Expression::InterpolatedString(string) => string.for_each_child_mut(visitor),
            Expression::Number(number) => number.for_each_child_mut(visitor),
            Expression::Parenthese(parenthese) => parenthese.for_each_child_mut(visitor),
            Expression::String(string) => string.for_each_child_mut(visitor),
            Expression::Table(table) => table.for_each_child_mut(visitor),
            Expression::Unary(unary) => unary.for_each_child_mut(visitor),
            Expression::TypeCast(type_cast) => type_cast.for_each_child_mut(visitor),
            Expression::TypeInstantiation(instantiation) => {
                instantiation.for_each_child_mut(visitor)
            }
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => {}
        }
    }
}

impl ReadOnlyChildren for BinaryExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.left());
        visitor.expression(self.right());
    }
}

impl MutableChildren for BinaryExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_left());
        visitor.expression(self.mutate_right());
    }
}

impl ReadOnlyChildren for FieldExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.prefix(self.get_prefix());
        visitor.identifier(self.get_field());
    }
}

impl MutableChildren for FieldExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.prefix(self.mutate_prefix());
        visitor.identifier(self.mutate_field());
    }
}

impl ReadOnlyChildren for FunctionExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.attributes(self.attributes());
        if let Some(generic_parameters) = self.get_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.get_block());
        for parameter in self.iter_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.get_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.get_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl MutableChildren for FunctionExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.attributes(self.mutate_attributes());
        if let Some(generic_parameters) = self.mutate_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        visitor.block(self.mutate_block());
        for parameter in self.iter_mut_parameters() {
            visitor.typed_identifier(parameter);
        }
        if let Some(variadic_type) = self.mutate_variadic_type() {
            visitor.function_variadic_type(variadic_type);
        }
        if let Some(return_type) = self.mutate_return_type() {
            visitor.function_return_type(return_type);
        }
    }
}

impl ReadOnlyChildren for IndexExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.prefix(self.get_prefix());
        visitor.expression(self.get_index());
    }
}

impl MutableChildren for IndexExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.prefix(self.mutate_prefix());
        visitor.expression(self.mutate_index());
    }
}

impl ReadOnlyChildren for ElseIfExpressionBranch {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_condition());
        visitor.expression(self.get_result());
    }
}

impl MutableChildren for ElseIfExpressionBranch {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_condition());
        visitor.expression(self.mutate_result());
    }
}

impl ReadOnlyChildren for IfExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_condition());
        visitor.expression(self.get_result());
        for branch in self.iter_branches() {
            branch.for_each_child(visitor);
        }
        visitor.expression(self.get_else_result());
    }
}

impl MutableChildren for IfExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_condition());
        visitor.expression(self.mutate_result());
        for branch in self.iter_mut_branches() {
            branch.for_each_child_mut(visitor);
        }
        visitor.expression(self.mutate_else_result());
    }
}

impl ReadOnlyChildren for InterpolationSegment {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        if let InterpolationSegment::Value(value) = self {
            value.for_each_child(visitor);
        }
    }
}

impl MutableChildren for InterpolationSegment {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        if let InterpolationSegment::Value(value) = self {
            value.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for ValueSegment {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_expression());
    }
}

impl MutableChildren for ValueSegment {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_expression());
    }
}

impl ReadOnlyChildren for InterpolatedStringExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for segment in self.iter_segments() {
            segment.for_each_child(visitor);
        }
    }
}

impl MutableChildren for InterpolatedStringExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for segment in self.iter_mut_segments() {
            segment.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for ParentheseExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.inner_expression());
    }
}

impl MutableChildren for ParentheseExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_inner_expression());
    }
}

impl ReadOnlyChildren for TableFieldEntry {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_field());
        visitor.expression(self.get_value());
    }
}

impl MutableChildren for TableFieldEntry {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_field());
        visitor.expression(self.mutate_value());
    }
}

impl ReadOnlyChildren for TableIndexEntry {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_key());
        visitor.expression(self.get_value());
    }
}

impl MutableChildren for TableIndexEntry {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_key());
        visitor.expression(self.mutate_value());
    }
}

impl ReadOnlyChildren for TableEntry {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            TableEntry::Field(entry) => entry.for_each_child(visitor),
            TableEntry::Index(entry) => entry.for_each_child(visitor),
            TableEntry::Value(value) => visitor.expression(value),
        }
    }
}

impl MutableChildren for TableEntry {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            TableEntry::Field(entry) => entry.for_each_child_mut(visitor),
            TableEntry::Index(entry) => entry.for_each_child_mut(visitor),
            TableEntry::Value(value) => visitor.expression(value),
        }
    }
}

impl ReadOnlyChildren for TableExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for entry in self.iter_entries() {
            entry.for_each_child(visitor);
        }
    }
}

impl MutableChildren for TableExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for entry in self.iter_mut_entries() {
            entry.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for UnaryExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_expression());
    }
}

impl MutableChildren for UnaryExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_expression());
    }
}

impl ReadOnlyChildren for TypeCastExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_expression());
        visitor.r#type(self.get_type());
    }
}

impl MutableChildren for TypeCastExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_expression());
        visitor.r#type(self.mutate_type());
    }
}

impl ReadOnlyChildren for TypeInstantiationExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.prefix(self.get_prefix());
        for r#type in self.iter_types() {
            visitor.r#type(r#type);
        }
    }
}

impl MutableChildren for TypeInstantiationExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.prefix(self.mutate_prefix());
        for r#type in self.iter_mut_types() {
            visitor.r#type(r#type);
        }
    }
}

impl ReadOnlyChildren for FunctionCall {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.prefix(self.get_prefix());
        if let Some(method) = self.get_method() {
            visitor.identifier(method);
        }
        for r#type in self.get_method_type_instantiation() {
            visitor.r#type(r#type);
        }
        visitor.arguments(self.get_arguments());
    }
}

impl MutableChildren for FunctionCall {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.prefix(self.mutate_prefix());
        if let Some(method) = self.mutate_method() {
            visitor.identifier(method);
        }
        for r#type in self.iter_mut_method_type_instantiation() {
            visitor.r#type(r#type);
        }
        visitor.arguments(self.mutate_arguments());
    }
}

impl ReadOnlyChildren for Prefix {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            Prefix::Call(call) => call.for_each_child(visitor),
            Prefix::Field(field) => field.for_each_child(visitor),
            Prefix::Identifier(identifier) => visitor.variable_expression(identifier),
            Prefix::Index(index) => index.for_each_child(visitor),
            Prefix::Parenthese(parenthese) => parenthese.for_each_child(visitor),
            Prefix::TypeInstantiation(instantiation) => instantiation.for_each_child(visitor),
        }
    }
}

impl MutableChildren for Prefix {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            Prefix::Call(call) => call.for_each_child_mut(visitor),
            Prefix::Field(field) => field.for_each_child_mut(visitor),
            Prefix::Identifier(identifier) => visitor.variable_expression(identifier),
            Prefix::Index(index) => index.for_each_child_mut(visitor),
            Prefix::Parenthese(parenthese) => parenthese.for_each_child_mut(visitor),
            Prefix::TypeInstantiation(instantiation) => instantiation.for_each_child_mut(visitor),
        }
    }
}

impl ReadOnlyChildren for Variable {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            Variable::Identifier(identifier) => visitor.variable_expression(identifier),
            Variable::Field(field) => field.for_each_child(visitor),
            Variable::Index(index) => index.for_each_child(visitor),
        }
    }
}

impl MutableChildren for Variable {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            Variable::Identifier(identifier) => visitor.variable_expression(identifier),
            Variable::Field(field) => field.for_each_child_mut(visitor),
            Variable::Index(index) => index.for_each_child_mut(visitor),
        }
    }
}

impl ReadOnlyChildren for TupleArguments {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for value in self.iter_values() {
            visitor.expression(value);
        }
    }
}

impl MutableChildren for TupleArguments {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for value in self.iter_mut_values() {
            visitor.expression(value);
        }
    }
}

impl ReadOnlyChildren for Arguments {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            Arguments::Tuple(tuple) => tuple.for_each_child(visitor),
            Arguments::String(string) => visitor.string_argument(string),
            Arguments::Table(table) => visitor.table(table),
        }
    }
}

impl MutableChildren for Arguments {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            Arguments::Tuple(tuple) => tuple.for_each_child_mut(visitor),
            Arguments::String(string) => visitor.string_argument(string),
            Arguments::Table(table) => visitor.table(table),
        }
    }
}

impl ReadOnlyChildren for NamedAttribute {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_identifier());
    }
}

impl MutableChildren for NamedAttribute {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_identifier());
    }
}

impl ReadOnlyChildren for AttributeGroupElement {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.name());
        if let Some(arguments) = self.get_arguments() {
            arguments.for_each_child(visitor);
        }
    }
}

impl MutableChildren for AttributeGroupElement {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_name());
        if let Some(arguments) = self.mutate_arguments() {
            arguments.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for AttributeGroup {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for element in self.iter_attributes() {
            element.for_each_child(visitor);
        }
    }
}

impl MutableChildren for AttributeGroup {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for element in self.iter_mut_attributes() {
            element.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for Attribute {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            Attribute::Name(named) => named.for_each_child(visitor),
            Attribute::Group(group) => group.for_each_child(visitor),
        }
    }
}

impl MutableChildren for Attribute {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            Attribute::Name(named) => named.for_each_child_mut(visitor),
            Attribute::Group(group) => group.for_each_child_mut(visitor),
        }
    }
}

impl ReadOnlyChildren for Attributes {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for attribute in self.iter_attributes() {
            attribute.for_each_child(visitor);
        }
    }
}

impl MutableChildren for Attributes {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for attribute in self.iter_mut_attributes() {
            attribute.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for AttributeTupleArguments {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for value in self.iter_values() {
            visitor.literal_argument(value);
        }
    }
}

impl MutableChildren for AttributeTupleArguments {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for value in self.iter_mut_values() {
            visitor.literal_argument(value);
        }
    }
}

impl ReadOnlyChildren for AttributeArguments {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            AttributeArguments::Tuple(tuple) => tuple.for_each_child(visitor),
            AttributeArguments::String(string) => visitor.string_expression(string),
            AttributeArguments::Table(table) => visitor.literal_table(table),
        }
    }
}

impl MutableChildren for AttributeArguments {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            AttributeArguments::Tuple(tuple) => tuple.for_each_child_mut(visitor),
            AttributeArguments::String(string) => visitor.string_expression(string),
            AttributeArguments::Table(table) => visitor.literal_table(table),
        }
    }
}

impl ReadOnlyChildren for LiteralTableFieldEntry {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_field());
        visitor.literal_expression(self.get_value());
    }
}

impl MutableChildren for LiteralTableFieldEntry {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_field());
        visitor.literal_expression(self.mutate_value());
    }
}

impl ReadOnlyChildren for LiteralTableEntry {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            LiteralTableEntry::Field(entry) => entry.for_each_child(visitor),
            LiteralTableEntry::Value(value) => visitor.literal_expression(value),
        }
    }
}

impl MutableChildren for LiteralTableEntry {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            LiteralTableEntry::Field(entry) => entry.for_each_child_mut(visitor),
            LiteralTableEntry::Value(value) => visitor.literal_expression(value),
        }
    }
}

impl ReadOnlyChildren for LiteralTable {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for entry in self.iter_entries() {
            entry.for_each_child(visitor);
        }
    }
}

impl MutableChildren for LiteralTable {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for entry in self.iter_mut_entries() {
            entry.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for LiteralExpression {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            LiteralExpression::Number(number) => visitor.number_expression(number),
            LiteralExpression::String(string) => visitor.string_expression(string),
            LiteralExpression::Table(table) => visitor.literal_table(table),
            LiteralExpression::True(_)
            | LiteralExpression::False(_)
            | LiteralExpression::Nil(_) => {}
        }
    }
}

impl MutableChildren for LiteralExpression {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            LiteralExpression::Number(number) => visitor.number_expression(number),
            LiteralExpression::String(string) => visitor.string_expression(string),
            LiteralExpression::Table(table) => visitor.literal_table(table),
            LiteralExpression::True(_)
            | LiteralExpression::False(_)
            | LiteralExpression::Nil(_) => {}
        }
    }
}

impl ReadOnlyChildren for Type {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            Type::Name(type_name) => visitor.type_name(type_name),
            Type::Field(field) => field.for_each_child(visitor),
            Type::Array(array) => array.for_each_child(visitor),
            Type::Table(table) => table.for_each_child(visitor),
            Type::TypeOf(expression_type) => expression_type.for_each_child(visitor),
            Type::Parenthese(parenthese) => parenthese.for_each_child(visitor),
            Type::Function(function) => function.for_each_child(visitor),
            Type::Optional(optional) => optional.for_each_child(visitor),
            Type::Intersection(intersection) => intersection.for_each_child(visitor),
            Type::Union(union) => union.for_each_child(visitor),
            Type::String(string) => string.for_each_child(visitor),
            Type::True(_) | Type::False(_) | Type::Nil(_) => {}
        }
    }
}

impl MutableChildren for Type {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            Type::Name(type_name) => type_name.for_each_child_mut(visitor),
            Type::Field(field) => field.for_each_child_mut(visitor),
            Type::Array(array) => array.for_each_child_mut(visitor),
            Type::Table(table) => table.for_each_child_mut(visitor),
            Type::TypeOf(expression_type) => expression_type.for_each_child_mut(visitor),
            Type::Parenthese(parenthese) => parenthese.for_each_child_mut(visitor),
            Type::Function(function) => function.for_each_child_mut(visitor),
            Type::Optional(optional) => optional.for_each_child_mut(visitor),
            Type::Intersection(intersection) => intersection.for_each_child_mut(visitor),
            Type::Union(union) => union.for_each_child_mut(visitor),
            Type::String(string) => string.for_each_child_mut(visitor),
            Type::True(_) | Type::False(_) | Type::Nil(_) => {}
        }
    }
}

impl ReadOnlyChildren for TypeParameter {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            TypeParameter::Type(r#type) => visitor.r#type(r#type),
            TypeParameter::TypePack(type_pack) => visitor.type_pack(type_pack),
            TypeParameter::VariadicTypePack(variadic) => visitor.variadic_type_pack(variadic),
            TypeParameter::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl MutableChildren for TypeParameter {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            TypeParameter::Type(r#type) => visitor.r#type(r#type),
            TypeParameter::TypePack(type_pack) => visitor.type_pack(type_pack),
            TypeParameter::VariadicTypePack(variadic) => visitor.variadic_type_pack(variadic),
            TypeParameter::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl ReadOnlyChildren for TypeParameters {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for parameter in self.iter() {
            parameter.for_each_child(visitor);
        }
    }
}

impl MutableChildren for TypeParameters {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for parameter in self.iter_mut() {
            parameter.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for TypeName {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_type_name());
        if let Some(type_parameters) = self.get_type_parameters() {
            type_parameters.for_each_child(visitor);
        }
    }
}

impl MutableChildren for TypeName {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_type_name());
        if let Some(type_parameters) = self.mutate_type_parameters() {
            type_parameters.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for TypeField {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_namespace());
        visitor.type_name(self.get_type_name());
    }
}

impl MutableChildren for TypeField {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_namespace());
        visitor.type_name(self.mutate_type_name());
    }
}

impl ReadOnlyChildren for ArrayType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.r#type(self.get_element_type());
    }
}

impl MutableChildren for ArrayType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.r#type(self.mutate_element_type());
    }
}

impl ReadOnlyChildren for TablePropertyType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_identifier());
        visitor.r#type(self.get_type());
    }
}

impl MutableChildren for TablePropertyType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_identifier());
        visitor.r#type(self.mutate_type());
    }
}

impl ReadOnlyChildren for TableLiteralPropertyType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.string_type(self.get_string());
        visitor.r#type(self.get_type());
    }
}

impl MutableChildren for TableLiteralPropertyType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.string_type(self.mutate_string());
        visitor.r#type(self.mutate_type());
    }
}

impl ReadOnlyChildren for TableIndexerType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.r#type(self.get_key_type());
        visitor.r#type(self.get_value_type());
    }
}

impl MutableChildren for TableIndexerType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.r#type(self.mutate_key_type());
        visitor.r#type(self.mutate_value_type());
    }
}

impl ReadOnlyChildren for TableEntryType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            TableEntryType::Property(property) => property.for_each_child(visitor),
            TableEntryType::Literal(literal) => literal.for_each_child(visitor),
            TableEntryType::Indexer(indexer) => indexer.for_each_child(visitor),
        }
    }
}

impl MutableChildren for TableEntryType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            TableEntryType::Property(property) => property.for_each_child_mut(visitor),
            TableEntryType::Literal(literal) => literal.for_each_child_mut(visitor),
            TableEntryType::Indexer(indexer) => indexer.for_each_child_mut(visitor),
        }
    }
}

impl ReadOnlyChildren for TableType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for entry in self.iter_entries() {
            entry.for_each_child(visitor);
        }
    }
}

impl MutableChildren for TableType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for entry in self.iter_mut_entries() {
            entry.for_each_child_mut(visitor);
        }
    }
}

impl ReadOnlyChildren for ExpressionType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.expression(self.get_expression());
    }
}

impl MutableChildren for ExpressionType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.expression(self.mutate_expression());
    }
}

impl ReadOnlyChildren for ParentheseType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.r#type(self.get_inner_type());
    }
}

impl MutableChildren for ParentheseType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.r#type(self.mutate_inner_type());
    }
}

impl ReadOnlyChildren for FunctionArgumentType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        if let Some(name) = self.get_name() {
            visitor.identifier(name);
        }
        visitor.r#type(self.get_type());
    }
}

impl MutableChildren for FunctionArgumentType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        if let Some(name) = self.mutate_name() {
            visitor.identifier(name);
        }
        visitor.r#type(self.mutate_type());
    }
}

impl ReadOnlyChildren for FunctionType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        if let Some(generic_parameters) = self.get_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        for argument in self.iter_arguments() {
            argument.for_each_child(visitor);
        }
        if let Some(variadic) = self.get_variadic_argument_type() {
            visitor.variadic_argument_type(variadic);
        }
        visitor.function_return_type(self.get_return_type());
    }
}

impl MutableChildren for FunctionType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        if let Some(generic_parameters) = self.mutate_generic_parameters() {
            visitor.generic_parameters(generic_parameters);
        }
        for argument in self.iter_mut_arguments() {
            argument.for_each_child_mut(visitor);
        }
        if let Some(variadic) = self.mutate_variadic_argument_type() {
            visitor.variadic_argument_type(variadic);
        }
        visitor.function_return_type(self.mutate_return_type());
    }
}

impl ReadOnlyChildren for OptionalType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.r#type(self.get_inner_type());
    }
}

impl MutableChildren for OptionalType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.r#type(self.mutate_inner_type());
    }
}

impl ReadOnlyChildren for IntersectionType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for r#type in self.iter_types() {
            visitor.r#type(r#type);
        }
    }
}

impl MutableChildren for IntersectionType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for r#type in self.iter_mut_types() {
            visitor.r#type(r#type);
        }
    }
}

impl ReadOnlyChildren for UnionType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for r#type in self.iter_types() {
            visitor.r#type(r#type);
        }
    }
}

impl MutableChildren for UnionType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for r#type in self.iter_mut_types() {
            visitor.r#type(r#type);
        }
    }
}

impl ReadOnlyChildren for StringType {
    fn for_each_child(&self, _visitor: &mut impl ChildVisitor) {}
}

impl MutableChildren for StringType {
    fn for_each_child_mut(&mut self, _visitor: &mut impl ChildVisitorMut) {}
}

impl ReadOnlyChildren for TypePack {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for r#type in self.iter() {
            visitor.r#type(r#type);
        }
        if let Some(variadic) = self.get_variadic_type() {
            visitor.variadic_argument_type(variadic);
        }
    }
}

impl MutableChildren for TypePack {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for r#type in self.iter_mut() {
            visitor.r#type(r#type);
        }
        if let Some(variadic) = self.mutate_variadic_type() {
            visitor.variadic_argument_type(variadic);
        }
    }
}

impl ReadOnlyChildren for VariadicTypePack {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.r#type(self.get_type());
    }
}

impl MutableChildren for VariadicTypePack {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.r#type(self.mutate_type());
    }
}

impl ReadOnlyChildren for GenericTypePack {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_name());
    }
}

impl MutableChildren for GenericTypePack {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self.mutate_name());
    }
}

impl ReadOnlyChildren for GenericParameters {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for identifier in self.iter_type_variable() {
            visitor.identifier(identifier);
        }
        for generic in self.iter_generic_type_pack() {
            visitor.generic_type_pack(generic);
        }
    }
}

impl MutableChildren for GenericParameters {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for identifier in self.iter_mut_type_variable() {
            visitor.identifier(identifier);
        }
        for generic in self.iter_mut_generic_type_pack() {
            visitor.generic_type_pack(generic);
        }
    }
}

impl ReadOnlyChildren for GenericTypePackDefault {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            GenericTypePackDefault::TypePack(type_pack) => visitor.type_pack(type_pack),
            GenericTypePackDefault::VariadicTypePack(variadic) => {
                visitor.variadic_type_pack(variadic)
            }
            GenericTypePackDefault::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl MutableChildren for GenericTypePackDefault {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            GenericTypePackDefault::TypePack(type_pack) => visitor.type_pack(type_pack),
            GenericTypePackDefault::VariadicTypePack(variadic) => {
                visitor.variadic_type_pack(variadic)
            }
            GenericTypePackDefault::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl ReadOnlyChildren for GenericParametersWithDefaults {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        for parameter in self.iter() {
            match parameter {
                GenericParameterRef::TypeVariable(identifier) => visitor.identifier(identifier),
                GenericParameterRef::TypeVariableWithDefault(parameter) => {
                    visitor.identifier(parameter.get_type_variable());
                    visitor.r#type(parameter.get_default_type());
                }
                GenericParameterRef::GenericTypePack(generic) => visitor.generic_type_pack(generic),
                GenericParameterRef::GenericTypePackWithDefault(parameter) => {
                    visitor.generic_type_pack(parameter.get_generic_type_pack());
                    parameter.get_default_type().for_each_child(visitor);
                }
            }
        }
    }
}

impl MutableChildren for GenericParametersWithDefaults {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        for parameter in self.iter_mut() {
            match parameter {
                GenericParameterMutRef::TypeVariable(identifier) => visitor.identifier(identifier),
                GenericParameterMutRef::TypeVariableWithDefault(parameter) => {
                    visitor.identifier(parameter.mutate_type_variable());
                    visitor.r#type(parameter.mutate_default_type());
                }
                GenericParameterMutRef::GenericTypePack(generic) => {
                    visitor.generic_type_pack(generic)
                }
                GenericParameterMutRef::GenericTypePackWithDefault(parameter) => {
                    visitor.generic_type_pack(parameter.mutate_generic_type_pack());
                    parameter.mutate_default_type().for_each_child_mut(visitor);
                }
            }
        }
    }
}

impl ReadOnlyChildren for FunctionVariadicType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            FunctionVariadicType::Type(r#type) => visitor.r#type(r#type),
            FunctionVariadicType::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl MutableChildren for FunctionVariadicType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            FunctionVariadicType::Type(r#type) => visitor.r#type(r#type),
            FunctionVariadicType::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl ReadOnlyChildren for FunctionReturnType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            FunctionReturnType::Type(r#type) => visitor.r#type(r#type),
            FunctionReturnType::TypePack(type_pack) => visitor.type_pack(type_pack),
            FunctionReturnType::VariadicTypePack(variadic) => visitor.variadic_type_pack(variadic),
            FunctionReturnType::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl MutableChildren for FunctionReturnType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            FunctionReturnType::Type(r#type) => visitor.r#type(r#type),
            FunctionReturnType::TypePack(type_pack) => visitor.type_pack(type_pack),
            FunctionReturnType::VariadicTypePack(variadic) => visitor.variadic_type_pack(variadic),
            FunctionReturnType::GenericTypePack(generic) => visitor.generic_type_pack(generic),
        }
    }
}

impl ReadOnlyChildren for VariadicArgumentType {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        match self {
            VariadicArgumentType::GenericTypePack(generic) => visitor.generic_type_pack(generic),
            VariadicArgumentType::VariadicTypePack(variadic) => {
                visitor.variadic_type_pack(variadic)
            }
        }
    }
}

impl MutableChildren for VariadicArgumentType {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        match self {
            VariadicArgumentType::GenericTypePack(generic) => visitor.generic_type_pack(generic),
            VariadicArgumentType::VariadicTypePack(variadic) => {
                visitor.variadic_type_pack(variadic)
            }
        }
    }
}

impl ReadOnlyChildren for TypedIdentifier {
    fn for_each_child(&self, visitor: &mut impl ChildVisitor) {
        visitor.identifier(self.get_identifier());
        if let Some(r#type) = self.get_type() {
            visitor.r#type(r#type);
        }
    }
}

impl MutableChildren for TypedIdentifier {
    fn for_each_child_mut(&mut self, visitor: &mut impl ChildVisitorMut) {
        visitor.identifier(self);
        if let Some(r#type) = self.mutate_type() {
            visitor.r#type(r#type);
        }
    }
}

impl ReadOnlyChildren for Identifier {
    fn for_each_child(&self, _visitor: &mut impl ChildVisitor) {}
}

impl MutableChildren for Identifier {
    fn for_each_child_mut(&mut self, _visitor: &mut impl ChildVisitorMut) {}
}

impl ReadOnlyChildren for NumberExpression {
    fn for_each_child(&self, _visitor: &mut impl ChildVisitor) {}
}

impl MutableChildren for NumberExpression {
    fn for_each_child_mut(&mut self, _visitor: &mut impl ChildVisitorMut) {}
}

impl ReadOnlyChildren for StringExpression {
    fn for_each_child(&self, _visitor: &mut impl ChildVisitor) {}
}

impl MutableChildren for StringExpression {
    fn for_each_child_mut(&mut self, _visitor: &mut impl ChildVisitorMut) {}
}
