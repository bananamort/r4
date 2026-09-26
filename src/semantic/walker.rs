//! AST traversal for the semantic analysis pass.
//!
//! This module is an extension of [`AnalyzerState`]: the walker drives the
//! coordinator while visiting statement, expression, variable and type nodes. It
//! pushes and pops lexical scopes, declares symbols, records reads/writes and
//! delegates labels/gotos to [`super::goto`].

use crate::nodes::*;

use super::analyzer::AnalyzerState;
use super::error::SemanticResult;
use super::goto::is_block_end_label;
use super::scope::{ScopeId, ScopeKind};
use super::symbol::SymbolKind;

impl AnalyzerState {
    /// Walks every statement of a block (including its optional last statement).
    ///
    /// Labels are recorded before being resolved so that forward gotos work;
    /// everything else is dispatched to [`Self::walk_statement`].
    pub(super) fn walk_block_statements(&mut self, block: &Block) -> SemanticResult<()> {
        self.assign_block_identity(block);
        self.hoist_type_declarations(block)?;

        // Record which scope this block introduces, so node-keyed queries can
        // map a block back to its scope.
        if let Some(identity) = block.get_identity() {
            let scope = self.current_scope();
            self.block_scopes.insert(identity.0, scope);
        }

        for (index, statement) in block.iter_statements().enumerate() {
            if matches!(statement, Statement::TypeDeclaration(_)) {
                if let Some(cursor) = self.type_declaration_cursors.last_mut() {
                    *cursor += 1;
                }
            }
            match statement {
                Statement::Label(label) => {
                    let is_block_end = is_block_end_label(block, index);
                    self.record_label(label, is_block_end)?;
                }
                statement => self.walk_statement(statement)?,
            }
        }

        if let Some(last_statement) = block.get_last_statement() {
            self.walk_last_statement(last_statement)?;
        }

        Ok(())
    }

    /// Declares every type alias in `block` in the current scope before the
    /// block's statements are walked.
    ///
    /// Luau type aliases are not order sensitive within a scope, so a type may
    /// reference one declared later in the same block (including mutually
    /// recursive aliases). Hoisting the names first makes those forward
    /// references resolve exactly like a later declaration would.
    fn hoist_type_declarations(&mut self, block: &Block) -> SemanticResult<()> {
        for statement in block.iter_statements() {
            if let Statement::TypeDeclaration(declaration) = statement {
                self.check_export_type_placement(declaration)?;
                self.declare_type_symbol(
                    declaration.get_name().get_name(),
                    declaration.is_exported(),
                    Some(declaration.get_name()),
                );
            }
        }
        Ok(())
    }

    fn enter_block(&mut self) -> ScopeId {
        self.push_scope(ScopeKind::Block)
    }

    fn enter_function(&mut self) -> ScopeId {
        let id = self.push_scope(ScopeKind::Function);
        self.function_stack.push(id);
        id
    }

    fn exit_function(&mut self) {
        let function = self
            .function_stack
            .pop()
            .expect("a function scope is always active while analyzing");
        debug_assert_eq!(function, self.current_scope());
        self.pop_scope();
    }

    fn walk_scoped_block(&mut self, block: &Block) -> SemanticResult<()> {
        self.enter_block();
        let result = self.walk_block_statements(block);
        self.pop_scope();
        result
    }

    fn walk_statement(&mut self, statement: &Statement) -> SemanticResult<()> {
        match statement {
            Statement::Assign(assign) => {
                for value in assign.iter_values() {
                    self.walk_expression(value)?;
                }
                for variable in assign.iter_variables() {
                    self.walk_variable_write(variable)?;
                }
            }
            Statement::Call(call) => self.walk_call(call)?,
            Statement::CompoundAssign(assign) => {
                self.walk_variable_read(assign.get_variable())?;
                self.walk_expression(assign.get_value())?;
                if let Variable::Identifier(identifier) = assign.get_variable() {
                    self.write_identifier(identifier)?;
                }
            }
            Statement::Do(do_statement) => {
                self.walk_scoped_block(do_statement.get_block())?;
            }
            Statement::Function(function) => self.walk_function_statement(function)?,
            Statement::GenericFor(generic_for) => {
                for expression in generic_for.iter_expressions() {
                    self.walk_expression(expression)?;
                }
                self.enter_block();
                for identifier in generic_for.iter_identifiers() {
                    self.declare_symbol(
                        identifier.get_name(),
                        SymbolKind::Iteration,
                        Some(identifier.get_identifier()),
                        identifier.get_attribute(),
                    )?;
                }
                for identifier in generic_for.iter_identifiers() {
                    if let Some(r#type) = identifier.get_type() {
                        self.walk_type(r#type)?;
                    }
                }
                let result = self.walk_block_statements(generic_for.get_block());
                self.pop_scope();
                result?;
            }
            Statement::Goto(goto_statement) => self.record_goto(goto_statement),
            Statement::If(if_statement) => {
                for branch in if_statement.iter_branches() {
                    self.walk_expression(branch.get_condition())?;
                    self.walk_scoped_block(branch.get_block())?;
                }
                if let Some(else_block) = if_statement.get_else_block() {
                    self.walk_scoped_block(else_block)?;
                }
            }
            Statement::Label(label) => {
                // Labels are normally recorded by `walk_block_statements`; handle
                // them here too so the walk is total.
                self.record_label(label, false)?;
            }
            Statement::LocalAssign(assign) => {
                // Values are resolved before the locals are declared, so that
                // `local x = x` binds the value to the outer/global `x`. Plain
                // identifier initializers are remembered so that passes can
                // reason about read-only aliases.
                let mut initializers = Vec::with_capacity(assign.values_len());
                for value in assign.iter_values() {
                    if let Expression::Identifier(identifier) = value {
                        initializers.push(Some(self.read_identifier(identifier)?));
                    } else {
                        self.walk_expression(value)?;
                        initializers.push(None);
                    }
                }
                for (index, variable) in assign.iter_variables().enumerate() {
                    // A Luau `const` declaration is read-only like a Lua 5.4
                    // `<const>` attribute; record it as such.
                    let attribute = variable.get_attribute().or_else(|| {
                        (assign.get_assignment_kind() == AssignmentKind::Const).then_some("const")
                    });
                    let symbol = self.declare_symbol(
                        variable.get_name(),
                        SymbolKind::Local,
                        Some(variable.get_identifier()),
                        attribute,
                    )?;
                    if let Some(Some(target)) = initializers.get(index) {
                        self.set_symbol_initializer(symbol, *target);
                    }
                }
                for variable in assign.iter_variables() {
                    if let Some(r#type) = variable.get_type() {
                        self.walk_type(r#type)?;
                    }
                }
            }
            Statement::LocalFunction(function) => {
                let attribute =
                    (function.get_assignment_kind() == AssignmentKind::Const).then_some("const");
                self.declare_symbol(
                    function.get_name(),
                    SymbolKind::Local,
                    Some(function.get_identifier()),
                    attribute,
                )?;
                self.enter_function();
                for parameter in function.iter_parameters() {
                    self.declare_symbol(
                        parameter.get_name(),
                        SymbolKind::Parameter,
                        Some(parameter.get_identifier()),
                        parameter.get_attribute(),
                    )?;
                }
                let names = generic_type_names(function.get_generic_parameters());
                self.check_generic_parameters(&names)?;
                self.push_type_parameters(names);
                let result = (|| {
                    self.walk_signature_types(
                        function.iter_parameters().collect(),
                        function.get_variadic_type(),
                        function.get_return_type(),
                    )?;
                    self.walk_block_statements(function.get_block())
                })();
                self.pop_type_parameters();
                self.exit_function();
                result?;
            }
            Statement::NumericFor(numeric_for) => {
                self.walk_expression(numeric_for.get_start())?;
                self.walk_expression(numeric_for.get_end())?;
                if let Some(step) = numeric_for.get_step() {
                    self.walk_expression(step)?;
                }
                self.enter_block();
                let identifier = numeric_for.get_identifier();
                self.declare_symbol(
                    identifier.get_name(),
                    SymbolKind::Iteration,
                    Some(identifier.get_identifier()),
                    identifier.get_attribute(),
                )?;
                if let Some(r#type) = identifier.get_type() {
                    self.walk_type(r#type)?;
                }
                let result = self.walk_block_statements(numeric_for.get_block());
                self.pop_scope();
                result?;
            }
            Statement::Repeat(repeat) => {
                self.enter_block();
                let result = (|| {
                    self.walk_block_statements(repeat.get_block())?;
                    self.walk_expression(repeat.get_condition())
                })();
                self.pop_scope();
                result?;
            }
            Statement::While(while_statement) => {
                self.walk_expression(while_statement.get_condition())?;
                self.walk_scoped_block(while_statement.get_block())?;
            }
            Statement::TypeDeclaration(declaration) => {
                self.walk_type_declaration(declaration)?;
            }
            Statement::TypeFunction(function) => {
                self.enter_function();
                for parameter in function.iter_parameters() {
                    self.declare_symbol(
                        parameter.get_name(),
                        SymbolKind::Parameter,
                        Some(parameter.get_identifier()),
                        parameter.get_attribute(),
                    )?;
                }
                let names = generic_type_names(function.get_generic_parameters());
                self.check_generic_parameters(&names)?;
                self.push_type_parameters(names);
                let result = (|| {
                    self.walk_signature_types(
                        function.iter_parameters().collect(),
                        function.get_variadic_type(),
                        function.get_return_type(),
                    )?;
                    self.walk_block_statements(function.get_block())
                })();
                self.pop_type_parameters();
                self.exit_function();
                result?;
            }
        }

        Ok(())
    }

    fn walk_last_statement(&mut self, statement: &LastStatement) -> SemanticResult<()> {
        if let LastStatement::Return(return_statement) = statement {
            for expression in return_statement.iter_expressions() {
                self.walk_expression(expression)?;
            }
        }
        Ok(())
    }

    fn walk_function_statement(&mut self, function: &FunctionStatement) -> SemanticResult<()> {
        let name = function.get_name();

        if name.get_field_names().is_empty() && name.get_method().is_none() {
            self.write_identifier(name.get_name())?;
        } else {
            self.read_identifier(name.get_name())?;
        }

        self.enter_function();
        if name.get_method().is_some() {
            self.declare_symbol("self", SymbolKind::Parameter, None, None)?;
        }
        for parameter in function.iter_parameters() {
            self.declare_symbol(
                parameter.get_name(),
                SymbolKind::Parameter,
                Some(parameter.get_identifier()),
                parameter.get_attribute(),
            )?;
        }
        let names = generic_type_names(function.get_generic_parameters());
        self.check_generic_parameters(&names)?;
        self.push_type_parameters(names);
        let result = (|| {
            self.walk_signature_types(
                function.iter_parameters().collect(),
                function.get_variadic_type(),
                function.get_return_type(),
            )?;
            self.walk_block_statements(function.get_block())
        })();
        self.pop_type_parameters();
        self.exit_function();
        result
    }

    fn walk_function_expression(&mut self, function: &FunctionExpression) -> SemanticResult<()> {
        self.enter_function();
        for parameter in function.iter_parameters() {
            self.declare_symbol(
                parameter.get_name(),
                SymbolKind::Parameter,
                Some(parameter.get_identifier()),
                parameter.get_attribute(),
            )?;
        }
        let names = generic_type_names(function.get_generic_parameters());
        self.check_generic_parameters(&names)?;
        self.push_type_parameters(names);
        let result = (|| {
            self.walk_signature_types(
                function.iter_parameters().collect(),
                function.get_variadic_type(),
                function.get_return_type(),
            )?;
            self.walk_block_statements(function.get_block())
        })();
        self.pop_type_parameters();
        self.exit_function();
        result
    }

    fn walk_variable_read(&mut self, variable: &Variable) -> SemanticResult<()> {
        match variable {
            Variable::Identifier(identifier) => {
                self.read_identifier(identifier)?;
                Ok(())
            }
            Variable::Field(field) => self.walk_prefix(field.get_prefix()),
            Variable::Index(index) => {
                self.walk_prefix(index.get_prefix())?;
                self.walk_expression(index.get_index())
            }
        }
    }

    fn walk_variable_write(&mut self, variable: &Variable) -> SemanticResult<()> {
        match variable {
            Variable::Identifier(identifier) => {
                self.write_identifier(identifier)?;
                Ok(())
            }
            Variable::Field(field) => self.walk_prefix(field.get_prefix()),
            Variable::Index(index) => {
                self.walk_prefix(index.get_prefix())?;
                self.walk_expression(index.get_index())
            }
        }
    }

    fn walk_prefix(&mut self, prefix: &Prefix) -> SemanticResult<()> {
        match prefix {
            Prefix::Call(call) => self.walk_call(call),
            Prefix::Field(field) => self.walk_prefix(field.get_prefix()),
            Prefix::Identifier(identifier) => self.read_identifier(identifier).map(|_| ()),
            Prefix::Index(index) => {
                self.walk_prefix(index.get_prefix())?;
                self.walk_expression(index.get_index())
            }
            Prefix::Parenthese(parenthese) => self.walk_expression(parenthese.inner_expression()),
            Prefix::TypeInstantiation(type_instantiation) => {
                self.walk_prefix(type_instantiation.get_prefix())?;
                for r#type in type_instantiation.iter_types() {
                    self.walk_type(r#type)?;
                }
                Ok(())
            }
        }
    }

    fn walk_call(&mut self, call: &FunctionCall) -> SemanticResult<()> {
        self.walk_prefix(call.get_prefix())?;

        match call.get_arguments() {
            Arguments::String(_) => Ok(()),
            Arguments::Table(table) => self.walk_table_expression(table),
            Arguments::Tuple(tuple) => {
                for expression in tuple.iter_values() {
                    self.walk_expression(expression)?;
                }
                Ok(())
            }
        }
    }

    fn walk_expression(&mut self, expression: &Expression) -> SemanticResult<()> {
        match expression {
            Expression::Binary(binary) => {
                self.walk_expression(binary.left())?;
                self.walk_expression(binary.right())
            }
            Expression::Call(call) => self.walk_call(call),
            Expression::Field(field) => self.walk_prefix(field.get_prefix()),
            Expression::Function(function) => self.walk_function_expression(function),
            Expression::Identifier(identifier) => self.read_identifier(identifier).map(|_| ()),
            Expression::If(if_expression) => {
                self.walk_expression(if_expression.get_condition())?;
                self.walk_expression(if_expression.get_result())?;
                for branch in if_expression.iter_branches() {
                    self.walk_expression(branch.get_condition())?;
                    self.walk_expression(branch.get_result())?;
                }
                self.walk_expression(if_expression.get_else_result())
            }
            Expression::Index(index) => {
                self.walk_prefix(index.get_prefix())?;
                self.walk_expression(index.get_index())
            }
            Expression::InterpolatedString(string) => {
                for segment in string.iter_segments() {
                    if let InterpolationSegment::Value(value) = segment {
                        self.walk_expression(value.get_expression())?;
                    }
                }
                Ok(())
            }
            Expression::Parenthese(parenthese) => {
                self.walk_expression(parenthese.inner_expression())
            }
            Expression::Table(table) => self.walk_table_expression(table),
            Expression::Unary(unary) => self.walk_expression(unary.get_expression()),
            Expression::TypeCast(type_cast) => {
                self.walk_expression(type_cast.get_expression())?;
                self.walk_type(type_cast.get_type())
            }
            Expression::TypeInstantiation(type_instantiation) => {
                self.walk_prefix(type_instantiation.get_prefix())?;
                for r#type in type_instantiation.iter_types() {
                    self.walk_type(r#type)?;
                }
                Ok(())
            }
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::String(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => Ok(()),
        }
    }

    fn walk_table_expression(&mut self, table: &TableExpression) -> SemanticResult<()> {
        for entry in table.iter_entries() {
            match entry {
                TableEntry::Field(field) => self.walk_expression(field.get_value())?,
                TableEntry::Index(index) => {
                    self.walk_expression(index.get_key())?;
                    self.walk_expression(index.get_value())?;
                }
                TableEntry::Value(value) => self.walk_expression(value)?,
            }
        }
        Ok(())
    }

    /// Walks the runtime expressions that can be embedded inside Luau types
    /// (`typeof(expr)`).
    fn walk_type(&mut self, r#type: &Type) -> SemanticResult<()> {
        match r#type {
            Type::TypeOf(expression_type) => self.walk_expression(expression_type.get_expression()),
            Type::Name(type_name) => self.walk_type_name(type_name),
            Type::Field(field) => self.walk_type_name(field.get_type_name()),
            Type::Array(array) => self.walk_type(array.get_element_type()),
            Type::Table(table) => {
                for entry in table.iter_entries() {
                    match entry {
                        TableEntryType::Property(property) => {
                            self.walk_type(property.get_type())?
                        }
                        TableEntryType::Literal(property) => self.walk_type(property.get_type())?,
                        TableEntryType::Indexer(indexer) => {
                            self.walk_type(indexer.get_key_type())?;
                            self.walk_type(indexer.get_value_type())?;
                        }
                    }
                }
                Ok(())
            }
            Type::Parenthese(parenthese) => self.walk_type(parenthese.get_inner_type()),
            Type::Function(function) => {
                for argument in function.iter_arguments() {
                    self.walk_type(argument.get_type())?;
                }
                if let Some(variadic) = function.get_variadic_argument_type() {
                    self.walk_variadic_argument_type(variadic)?;
                }
                self.walk_function_return_type(function.get_return_type())
            }
            Type::Optional(optional) => self.walk_type(optional.get_inner_type()),
            Type::Intersection(intersection) => {
                for next_type in intersection.iter_types() {
                    self.walk_type(next_type)?;
                }
                Ok(())
            }
            Type::Union(union) => {
                for next_type in union.iter_types() {
                    self.walk_type(next_type)?;
                }
                Ok(())
            }
            Type::True(_) | Type::False(_) | Type::Nil(_) | Type::String(_) => Ok(()),
        }
    }

    fn walk_type_name(&mut self, type_name: &TypeName) -> SemanticResult<()> {
        self.resolve_type_occurrence(type_name.get_type_name())?;

        if let Some(parameters) = type_name.get_type_parameters() {
            for parameter in parameters.iter() {
                match parameter {
                    TypeParameter::Type(next_type) => self.walk_type(next_type)?,
                    TypeParameter::TypePack(type_pack) => self.walk_type_pack(type_pack)?,
                    TypeParameter::VariadicTypePack(variadic) => {
                        self.walk_type(variadic.get_type())?
                    }
                    TypeParameter::GenericTypePack(pack) => {
                        self.check_type_occurrence(pack.get_name())?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Walks a type declaration: the alias was already declared by
    /// [`Self::hoist_type_declarations`], so this records a bare alias and
    /// walks the generic defaults and the declared type.
    fn walk_type_declaration(
        &mut self,
        declaration: &TypeDeclarationStatement,
    ) -> SemanticResult<()> {
        let id = declaration
            .get_name()
            .get_binding()
            .map(|binding| crate::nodes::TypeId(binding.index()))
            .expect("type declarations are hoisted before the block is walked");

        // Record a bare alias such as `type A = B` so that alias chains can be
        // expanded without re-analyzing the tree. The target is already
        // declared because every type in the block is hoisted first. A generic
        // alias (`type A<T> = ...`) has no single expansion and its
        // right-hand side may reference its own parameters, so it is not an
        // alias of anything.
        if declaration.get_generic_parameters().is_none() {
            if let crate::nodes::Type::Name(type_name) = declaration.get_type() {
                let target_name = type_name.get_type_name().get_name();
                if !type_name.has_type_parameters() && !self.is_generic_type_parameter(target_name)
                {
                    if let Some(target) = self.lookup_type_name(target_name) {
                        if target != id {
                            self.types[id.index()].alias_of = Some(target);
                        }
                    }
                }
            }
        }

        let mut names = Vec::new();
        let mut defaults = Vec::new();
        if let Some(parameters) = declaration.get_generic_parameters() {
            for parameter in parameters.iter() {
                match parameter {
                    GenericParameterRef::TypeVariable(identifier) => {
                        names.push(identifier.get_name().clone());
                    }
                    GenericParameterRef::TypeVariableWithDefault(variable) => {
                        names.push(variable.get_type_variable().get_name().clone());
                        defaults.push(variable.get_default_type());
                    }
                    GenericParameterRef::GenericTypePack(pack) => {
                        names.push(pack.get_name().get_name().clone());
                    }
                    GenericParameterRef::GenericTypePackWithDefault(pack) => {
                        names.push(pack.get_generic_type_pack().get_name().get_name().clone());
                    }
                }
            }
        }

        self.check_generic_parameters(&names)?;
        self.push_type_parameters(names);
        let result = (|| {
            for default in defaults {
                self.walk_type(default)?;
            }
            self.walk_type(declaration.get_type())
        })();
        self.pop_type_parameters();
        result
    }

    fn walk_type_pack(&mut self, type_pack: &TypePack) -> SemanticResult<()> {
        for r#type in type_pack.iter() {
            self.walk_type(r#type)?;
        }
        Ok(())
    }

    /// Walks the type annotations of a function signature.
    ///
    /// The caller is responsible for pushing the function's generic type
    /// parameters before calling this (and for the body).
    fn walk_signature_types<'b>(
        &mut self,
        parameters: Vec<&'b TypedIdentifier>,
        variadic_type: Option<&'b FunctionVariadicType>,
        return_type: Option<&'b FunctionReturnType>,
    ) -> SemanticResult<()> {
        for parameter in parameters {
            if let Some(r#type) = parameter.get_type() {
                self.walk_type(r#type)?;
            }
        }
        if let Some(variadic_type) = variadic_type {
            self.walk_function_variadic_type(variadic_type)?;
        }
        if let Some(return_type) = return_type {
            self.walk_function_return_type(return_type)?;
        }
        Ok(())
    }

    fn walk_variadic_argument_type(
        &mut self,
        variadic: &VariadicArgumentType,
    ) -> SemanticResult<()> {
        match variadic {
            VariadicArgumentType::VariadicTypePack(pack) => self.walk_type(pack.get_type()),
            VariadicArgumentType::GenericTypePack(pack) => {
                self.check_type_occurrence(pack.get_name())
            }
        }
    }

    fn walk_function_return_type(
        &mut self,
        return_type: &FunctionReturnType,
    ) -> SemanticResult<()> {
        match return_type {
            FunctionReturnType::Type(r#type) => self.walk_type(r#type),
            FunctionReturnType::VariadicTypePack(pack) => self.walk_type(pack.get_type()),
            FunctionReturnType::TypePack(type_pack) => self.walk_type_pack(type_pack),
            FunctionReturnType::GenericTypePack(pack) => {
                self.check_type_occurrence(pack.get_name())
            }
        }
    }

    fn walk_function_variadic_type(
        &mut self,
        variadic: &FunctionVariadicType,
    ) -> SemanticResult<()> {
        match variadic {
            FunctionVariadicType::Type(r#type) => self.walk_type(r#type),
            FunctionVariadicType::GenericTypePack(pack) => {
                self.check_type_occurrence(pack.get_name())
            }
        }
    }
}

/// Returns the names of a function's generic type parameters.
fn generic_type_names(parameters: Option<&GenericParameters>) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(parameters) = parameters {
        for identifier in parameters.iter_type_variable() {
            names.push(identifier.get_name().clone());
        }
        for pack in parameters.iter_generic_type_pack() {
            names.push(pack.get_name().get_name().clone());
        }
    }
    names
}
