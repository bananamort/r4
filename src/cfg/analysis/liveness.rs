use std::fmt;

use bitvec::prelude::*;

use crate::cfg::{BlockId, ControlFlowGraph, Terminator};
use crate::nodes::*;
use crate::semantic::{SemanticModel, SymbolId};

/// An error produced by liveness analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LivenessError {
    /// An identifier is not bound in the semantic model. This means the CFG
    /// was not built from the analyzed AST, or the AST was mutated after the
    /// analysis.
    UnresolvedIdentifier {
        /// The name of the unresolved identifier.
        name: String,
        /// The source line of the identifier, if known.
        line: Option<usize>,
    },
}

impl fmt::Display for LivenessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LivenessError::UnresolvedIdentifier { name, line } => match line {
                Some(line) => write!(
                    f,
                    "line {}: identifier `{}` is not bound in the semantic model",
                    line, name
                ),
                None => write!(
                    f,
                    "identifier `{}` is not bound in the semantic model",
                    name
                ),
            },
        }
    }
}

impl std::error::Error for LivenessError {}

/// Backwards live variable analysis over a [`ControlFlowGraph`].
///
/// For each block the analysis computes:
///
/// * `live_in`: symbols that are read before being written on some path starting
///   at the block,
/// * `live_out`: symbols live at the end of the block (union of the successors'
///   `live_in` sets),
/// * `uses`: symbols read before any write inside the block (upward exposed uses),
/// * `defs`: symbols written inside the block.
///
/// All sets are bitsets indexed by [`SymbolId`].
#[derive(Clone, Debug)]
pub struct Liveness {
    /// Number of symbols tracked (the semantic model's symbol count).
    pub symbol_count: usize,
    /// Live-in sets, indexed by block.
    pub live_in: Vec<BitVec>,
    /// Live-out sets, indexed by block.
    pub live_out: Vec<BitVec>,
    /// Upward exposed uses per block.
    pub uses: Vec<BitVec>,
    /// Definitions per block.
    pub defs: Vec<BitVec>,
    /// Reads performed by each statement, indexed by block and statement.
    /// Unlike `uses`, this includes reads of symbols defined earlier in the
    /// same block.
    pub statement_uses: Vec<Vec<BitVec>>,
    /// Definitions introduced by each statement, indexed by block and
    /// statement.
    pub statement_defs: Vec<Vec<BitVec>>,
    /// Reads performed by the block expressions and terminator (everything
    /// read after the last statement).
    pub terminator_uses: Vec<BitVec>,
}

impl Liveness {
    /// Runs liveness analysis on a graph.
    ///
    /// Identifiers are resolved through the structural bindings recorded by
    /// semantic analysis, so the AST does not need to be parsed with token
    /// preservation.
    pub fn analyze(cfg: &ControlFlowGraph, model: &SemanticModel) -> Result<Self, LivenessError> {
        let symbol_count = model.symbols.len();
        let block_count = cfg.blocks.len();

        let mut uses = Vec::with_capacity(block_count);
        let mut defs = Vec::with_capacity(block_count);
        let mut statement_uses = Vec::with_capacity(block_count);
        let mut statement_defs = Vec::with_capacity(block_count);
        let mut terminator_uses = Vec::with_capacity(block_count);

        for block in &cfg.blocks {
            let mut analyzer = BlockAnalyzer::new(model, symbol_count);
            let mut block_statement_uses = Vec::with_capacity(block.statements.len());
            let mut block_statement_defs = Vec::with_capacity(block.statements.len());

            for statement in &block.statements {
                analyzer.current_reads.fill(false);
                analyzer.current_defs.fill(false);
                analyzer.read_statement(statement)?;
                block_statement_uses.push(analyzer.current_reads.clone());
                block_statement_defs.push(analyzer.current_defs.clone());
            }

            analyzer.current_reads.fill(false);
            for expression in &block.expressions {
                analyzer.read_expression(expression)?;
            }
            analyzer.read_terminator(&block.terminator)?;
            let tail_uses = analyzer.current_reads.clone();

            uses.push(analyzer.uses);
            defs.push(analyzer.defs);
            statement_uses.push(block_statement_uses);
            statement_defs.push(block_statement_defs);
            terminator_uses.push(tail_uses);
        }

        let mut live_in = vec![BitVec::repeat(false, symbol_count); block_count];
        let mut live_out = vec![BitVec::repeat(false, symbol_count); block_count];

        // Reuse scratch buffers instead of allocating two bitsets per block on
        // every fixpoint iteration.
        let mut out = BitVec::repeat(false, symbol_count);
        let mut new_in = BitVec::repeat(false, symbol_count);

        loop {
            let mut changed = false;

            for index in (0..block_count).rev() {
                out.fill(false);
                for &successor in &cfg.blocks[index].successors {
                    for bit in live_in[successor.index()].iter_ones() {
                        out.set(bit, true);
                    }
                }

                new_in.clone_from(&uses[index]);
                for bit in out.iter_ones() {
                    if !defs[index][bit] {
                        new_in.set(bit, true);
                    }
                }

                if new_in != live_in[index] || out != live_out[index] {
                    live_in[index].clone_from(&new_in);
                    live_out[index].clone_from(&out);
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        Ok(Self {
            symbol_count,
            live_in,
            live_out,
            uses,
            defs,
            statement_uses,
            statement_defs,
            terminator_uses,
        })
    }

    /// Returns whether a symbol is live at the entry of a block.
    #[inline]
    pub fn is_live_in(&self, block: BlockId, symbol: SymbolId) -> bool {
        self.live_in[block.index()][symbol.index()]
    }

    /// Returns whether a symbol is live at the exit of a block.
    #[inline]
    pub fn is_live_out(&self, block: BlockId, symbol: SymbolId) -> bool {
        self.live_out[block.index()][symbol.index()]
    }

    /// Iterates over the symbols live at the entry of a block.
    pub fn live_in_symbols(&self, block: BlockId) -> impl Iterator<Item = SymbolId> + '_ {
        self.live_in[block.index()]
            .iter_ones()
            .map(|index| SymbolId(index as u32))
    }

    /// Iterates over the symbols live at the exit of a block.
    pub fn live_out_symbols(&self, block: BlockId) -> impl Iterator<Item = SymbolId> + '_ {
        self.live_out[block.index()]
            .iter_ones()
            .map(|index| SymbolId(index as u32))
    }
}

struct BlockAnalyzer<'a> {
    model: &'a SemanticModel,
    uses: BitVec,
    defs: BitVec,
    defined: BitVec,
    /// Every read, including reads of symbols that were already defined in the
    /// block. Used for statement-level data-flow queries such as dead stores.
    reads: BitVec,
    /// Reads recorded by the statement currently being processed.
    current_reads: BitVec,
    /// Definitions recorded by the statement currently being processed.
    current_defs: BitVec,
}

impl<'a> BlockAnalyzer<'a> {
    fn new(model: &'a SemanticModel, symbol_count: usize) -> Self {
        Self {
            model,
            uses: BitVec::repeat(false, symbol_count),
            defs: BitVec::repeat(false, symbol_count),
            defined: BitVec::repeat(false, symbol_count),
            reads: BitVec::repeat(false, symbol_count),
            current_reads: BitVec::repeat(false, symbol_count),
            current_defs: BitVec::repeat(false, symbol_count),
        }
    }

    fn resolve(&self, identifier: &Identifier) -> Result<SymbolId, LivenessError> {
        self.model.resolve_identifier(identifier).ok_or_else(|| {
            LivenessError::UnresolvedIdentifier {
                name: identifier.get_name().clone(),
                line: identifier
                    .get_token()
                    .and_then(|token| token.get_line_number()),
            }
        })
    }

    fn read_identifier(&mut self, identifier: &Identifier) -> Result<(), LivenessError> {
        let symbol = self.resolve(identifier)?;
        self.reads.set(symbol.index(), true);
        self.current_reads.set(symbol.index(), true);
        if !self.defined[symbol.index()] {
            self.uses.set(symbol.index(), true);
        }
        Ok(())
    }

    fn define_identifier(&mut self, identifier: &Identifier) -> Result<(), LivenessError> {
        let symbol = self.resolve(identifier)?;
        self.defs.set(symbol.index(), true);
        self.current_defs.set(symbol.index(), true);
        self.defined.set(symbol.index(), true);
        Ok(())
    }

    fn define_symbol(&mut self, symbol: SymbolId) {
        self.defs.set(symbol.index(), true);
        self.defined.set(symbol.index(), true);
    }

    fn read_variable(&mut self, variable: &Variable) -> Result<(), LivenessError> {
        match variable {
            Variable::Identifier(identifier) => self.read_identifier(identifier),
            Variable::Field(field) => self.read_prefix(field.get_prefix()),
            Variable::Index(index) => {
                self.read_prefix(index.get_prefix())?;
                self.read_expression(index.get_index())
            }
        }
    }

    fn write_variable(&mut self, variable: &Variable) -> Result<(), LivenessError> {
        match variable {
            Variable::Identifier(identifier) => self.define_identifier(identifier),
            Variable::Field(field) => self.read_prefix(field.get_prefix()),
            Variable::Index(index) => {
                self.read_prefix(index.get_prefix())?;
                self.read_expression(index.get_index())
            }
        }
    }

    fn read_prefix(&mut self, prefix: &Prefix) -> Result<(), LivenessError> {
        match prefix {
            Prefix::Call(call) => self.read_call(call),
            Prefix::Field(field) => self.read_prefix(field.get_prefix()),
            Prefix::Identifier(identifier) => self.read_identifier(identifier),
            Prefix::Index(index) => {
                self.read_prefix(index.get_prefix())?;
                self.read_expression(index.get_index())
            }
            Prefix::Parenthese(parenthese) => self.read_expression(parenthese.inner_expression()),
            Prefix::TypeInstantiation(type_instantiation) => {
                self.read_prefix(type_instantiation.get_prefix())?;
                self.read_types(type_instantiation.iter_types())
            }
        }
    }

    fn read_call(&mut self, call: &FunctionCall) -> Result<(), LivenessError> {
        self.read_prefix(call.get_prefix())?;

        match call.get_arguments() {
            Arguments::String(_) => Ok(()),
            Arguments::Table(table) => self.read_table(table),
            Arguments::Tuple(tuple) => {
                for expression in tuple.iter_values() {
                    self.read_expression(expression)?;
                }
                Ok(())
            }
        }
    }

    fn read_expression(&mut self, expression: &Expression) -> Result<(), LivenessError> {
        match expression {
            Expression::Binary(binary) => {
                self.read_expression(binary.left())?;
                self.read_expression(binary.right())
            }
            Expression::Call(call) => self.read_call(call),
            Expression::Field(field) => self.read_prefix(field.get_prefix()),
            // Function bodies run later; their internal accesses are not part of
            // the enclosing block's liveness.
            Expression::Function(_) => Ok(()),
            Expression::Identifier(identifier) => self.read_identifier(identifier),
            Expression::If(if_expression) => {
                self.read_expression(if_expression.get_condition())?;
                self.read_expression(if_expression.get_result())?;
                for branch in if_expression.iter_branches() {
                    self.read_expression(branch.get_condition())?;
                    self.read_expression(branch.get_result())?;
                }
                self.read_expression(if_expression.get_else_result())
            }
            Expression::Index(index) => {
                self.read_prefix(index.get_prefix())?;
                self.read_expression(index.get_index())
            }
            Expression::InterpolatedString(string) => {
                for segment in string.iter_segments() {
                    if let InterpolationSegment::Value(value) = segment {
                        self.read_expression(value.get_expression())?;
                    }
                }
                Ok(())
            }
            Expression::Parenthese(parenthese) => {
                self.read_expression(parenthese.inner_expression())
            }
            Expression::Table(table) => self.read_table(table),
            Expression::Unary(unary) => self.read_expression(unary.get_expression()),
            Expression::TypeCast(type_cast) => {
                self.read_expression(type_cast.get_expression())?;
                self.read_type(type_cast.get_type())
            }
            Expression::TypeInstantiation(type_instantiation) => {
                self.read_prefix(type_instantiation.get_prefix())?;
                self.read_types(type_instantiation.iter_types())
            }
            Expression::False(_)
            | Expression::Nil(_)
            | Expression::Number(_)
            | Expression::String(_)
            | Expression::True(_)
            | Expression::VariableArguments(_) => Ok(()),
        }
    }

    fn read_table(&mut self, table: &TableExpression) -> Result<(), LivenessError> {
        for entry in table.iter_entries() {
            match entry {
                TableEntry::Field(field) => self.read_expression(field.get_value())?,
                TableEntry::Index(index) => {
                    self.read_expression(index.get_key())?;
                    self.read_expression(index.get_value())?;
                }
                TableEntry::Value(value) => self.read_expression(value)?,
            }
        }
        Ok(())
    }

    fn read_types<'b>(
        &mut self,
        types: impl Iterator<Item = &'b Type>,
    ) -> Result<(), LivenessError> {
        for r#type in types {
            self.read_type(r#type)?;
        }
        Ok(())
    }

    fn read_type(&mut self, r#type: &Type) -> Result<(), LivenessError> {
        match r#type {
            Type::TypeOf(expression_type) => self.read_expression(expression_type.get_expression()),
            Type::Name(type_name) => self.read_type_name(type_name),
            Type::Field(field) => self.read_type_name(field.get_type_name()),
            Type::Array(array) => self.read_type(array.get_element_type()),
            Type::Table(table) => {
                for entry in table.iter_entries() {
                    match entry {
                        TableEntryType::Property(property) => {
                            self.read_type(property.get_type())?
                        }
                        TableEntryType::Literal(property) => self.read_type(property.get_type())?,
                        TableEntryType::Indexer(indexer) => {
                            self.read_type(indexer.get_key_type())?;
                            self.read_type(indexer.get_value_type())?;
                        }
                    }
                }
                Ok(())
            }
            Type::Parenthese(parenthese) => self.read_type(parenthese.get_inner_type()),
            Type::Function(function) => {
                for argument in function.iter_arguments() {
                    self.read_type(argument.get_type())?;
                }
                if let Some(variadic) = function.get_variadic_argument_type() {
                    self.read_variadic_argument_type(variadic)?;
                }
                self.read_function_return_type(function.get_return_type())
            }
            Type::Optional(optional) => self.read_type(optional.get_inner_type()),
            Type::Intersection(intersection) => {
                for next_type in intersection.iter_types() {
                    self.read_type(next_type)?;
                }
                Ok(())
            }
            Type::Union(union) => {
                for next_type in union.iter_types() {
                    self.read_type(next_type)?;
                }
                Ok(())
            }
            Type::True(_) | Type::False(_) | Type::Nil(_) | Type::String(_) => Ok(()),
        }
    }

    fn read_type_name(&mut self, type_name: &TypeName) -> Result<(), LivenessError> {
        if let Some(parameters) = type_name.get_type_parameters() {
            for parameter in parameters.iter() {
                if let TypeParameter::Type(next_type) = parameter {
                    self.read_type(next_type)?;
                }
            }
        }
        Ok(())
    }

    fn read_variadic_argument_type(
        &mut self,
        variadic: &VariadicArgumentType,
    ) -> Result<(), LivenessError> {
        match variadic {
            VariadicArgumentType::VariadicTypePack(pack) => self.read_type(pack.get_type()),
            VariadicArgumentType::GenericTypePack(_) => Ok(()),
        }
    }

    fn read_function_return_type(
        &mut self,
        return_type: &FunctionReturnType,
    ) -> Result<(), LivenessError> {
        match return_type {
            FunctionReturnType::Type(r#type) => self.read_type(r#type),
            FunctionReturnType::VariadicTypePack(pack) => self.read_type(pack.get_type()),
            FunctionReturnType::TypePack(_) | FunctionReturnType::GenericTypePack(_) => Ok(()),
        }
    }

    fn read_last_statement(&mut self, statement: &LastStatement) -> Result<(), LivenessError> {
        if let LastStatement::Return(return_statement) = statement {
            for expression in return_statement.iter_expressions() {
                self.read_expression(expression)?;
            }
        }
        Ok(())
    }

    fn read_terminator(&mut self, terminator: &Terminator) -> Result<(), LivenessError> {
        match terminator {
            Terminator::Branch { condition, .. } => self.read_expression(condition),
            Terminator::ForLoop { defines, .. } => {
                for &symbol in defines {
                    self.define_symbol(symbol);
                }
                Ok(())
            }
            Terminator::Return(Some(statement)) => self.read_last_statement(statement),
            Terminator::Jump(_)
            | Terminator::Return(None)
            | Terminator::Break(_)
            | Terminator::Continue(_)
            | Terminator::Unreachable => Ok(()),
        }
    }

    fn read_statement(&mut self, statement: &Statement) -> Result<(), LivenessError> {
        match statement {
            Statement::Assign(assign) => {
                for value in assign.iter_values() {
                    self.read_expression(value)?;
                }
                for variable in assign.iter_variables() {
                    self.write_variable(variable)?;
                }
                Ok(())
            }
            Statement::Call(call) => self.read_call(call),
            Statement::CompoundAssign(assign) => {
                self.read_variable(assign.get_variable())?;
                self.read_expression(assign.get_value())?;
                self.write_variable(assign.get_variable())
            }
            Statement::Do(do_statement) => self.read_block(do_statement.get_block()),
            Statement::Function(function) => {
                let name = function.get_name();
                if name.get_field_names().is_empty() && name.get_method().is_none() {
                    self.define_identifier(name.get_name())
                } else {
                    self.read_identifier(name.get_name())
                }
            }
            Statement::GenericFor(generic_for) => {
                for expression in generic_for.iter_expressions() {
                    self.read_expression(expression)?;
                }
                for identifier in generic_for.iter_identifiers() {
                    self.define_identifier(identifier.get_identifier())?;
                }
                self.read_block(generic_for.get_block())
            }
            Statement::Goto(_) | Statement::Label(_) => Ok(()),
            Statement::If(if_statement) => {
                for branch in if_statement.iter_branches() {
                    self.read_expression(branch.get_condition())?;
                    self.read_block(branch.get_block())?;
                }
                if let Some(else_block) = if_statement.get_else_block() {
                    self.read_block(else_block)?;
                }
                Ok(())
            }
            Statement::LocalAssign(assign) => {
                for value in assign.iter_values() {
                    self.read_expression(value)?;
                }
                for variable in assign.iter_variables() {
                    self.define_identifier(variable.get_identifier())?;
                }
                Ok(())
            }
            Statement::LocalFunction(function) => self.define_identifier(function.get_identifier()),
            Statement::NumericFor(numeric_for) => {
                self.read_expression(numeric_for.get_start())?;
                self.read_expression(numeric_for.get_end())?;
                if let Some(step) = numeric_for.get_step() {
                    self.read_expression(step)?;
                }
                self.define_identifier(numeric_for.get_identifier().get_identifier())?;
                self.read_block(numeric_for.get_block())
            }
            Statement::Repeat(repeat) => {
                self.read_block(repeat.get_block())?;
                self.read_expression(repeat.get_condition())
            }
            Statement::While(while_statement) => {
                self.read_expression(while_statement.get_condition())?;
                self.read_block(while_statement.get_block())
            }
            Statement::TypeDeclaration(declaration) => self.read_type(declaration.get_type()),
            // Type functions execute at type-check time only.
            Statement::TypeFunction(_) => Ok(()),
        }
    }

    fn read_block(&mut self, block: &Block) -> Result<(), LivenessError> {
        for statement in block.iter_statements() {
            self.read_statement(statement)?;
        }
        if let Some(last_statement) = block.get_last_statement() {
            self.read_last_statement(last_statement)?;
        }
        Ok(())
    }
}
