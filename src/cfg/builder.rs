use std::collections::HashMap;
use std::fmt;

use crate::nodes::{Block, GotoStatement, IfStatement, LabelStatement, LastStatement, Statement};
use crate::semantic::{GotoBinding, LabelId, ScopeId, SemanticModel, SymbolId};

use super::block::{BasicBlock, BlockId, Terminator};
use super::graph::ControlFlowGraph;
use super::provenance::{ConditionKind, ConditionOrigin, StatementOrigin};

/// An error produced while lowering an AST into a control flow graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfgError {
    /// The precise reason of the error.
    pub kind: CfgErrorKind,
    /// The source line of the offending construct, when token data is available.
    pub line: Option<usize>,
}

/// The different errors that can be reported while building a CFG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgErrorKind {
    /// A `break` appears outside of any loop.
    BreakOutsideLoop,
    /// A `continue` appears outside of any loop.
    ContinueOutsideLoop,
    /// The semantic model contained fewer goto/label records than the AST
    /// statements being lowered.
    MissingSemanticRecord {
        /// The kind of record that is missing (`goto` or `label`).
        kind: &'static str,
        /// The name of the offending statement.
        name: String,
    },
    /// A goto or label statement does not match the next semantic record.
    SemanticMismatch {
        /// The kind of record that mismatched.
        kind: &'static str,
        /// The name of the statement being lowered.
        name: String,
        /// The name recorded in the semantic model.
        recorded: String,
    },
    /// A goto remained unresolved after the whole function was lowered.
    UnresolvedGoto {
        /// The name of the unresolved goto.
        name: String,
    },
    /// A block in the AST has no structural identity, so the semantic model
    /// does not match the tree that is being lowered.
    MissingBlockIdentity,
    /// An identifier in a loop header is not bound in the semantic model.
    UnresolvedIdentifier {
        /// The name of the unresolved identifier.
        name: String,
    },
}

impl fmt::Display for CfgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(f, "line {}: ", line)?;
        }
        match &self.kind {
            CfgErrorKind::BreakOutsideLoop => write!(f, "break outside of a loop"),
            CfgErrorKind::ContinueOutsideLoop => write!(f, "continue outside of a loop"),
            CfgErrorKind::MissingSemanticRecord { kind, name } => write!(
                f,
                "missing semantic {} record for `{}`; was the AST analyzed?",
                kind, name
            ),
            CfgErrorKind::SemanticMismatch {
                kind,
                name,
                recorded,
            } => write!(
                f,
                "semantic {} record `{}` does not match statement `{}`",
                kind, recorded, name
            ),
            CfgErrorKind::UnresolvedGoto { name } => {
                write!(f, "unresolved goto `{}`", name)
            }
            CfgErrorKind::MissingBlockIdentity => write!(
                f,
                "a block has no structural identity; the semantic model does not match the AST"
            ),
            CfgErrorKind::UnresolvedIdentifier { name } => write!(
                f,
                "identifier `{}` is not bound in the semantic model",
                name
            ),
        }
    }
}

impl std::error::Error for CfgError {}

#[derive(Copy, Clone, Debug)]
struct LoopContext {
    break_target: BlockId,
    continue_target: BlockId,
}

/// Lowers an AST block and its [`SemanticModel`] into a [`ControlFlowGraph`].
pub struct CfgBuilder<'a> {
    model: &'a SemanticModel,
    function_scope: ScopeId,
    blocks: Vec<BasicBlock>,
    entry: BlockId,
    exit: BlockId,
    current: Option<BlockId>,
    loops: Vec<LoopContext>,
    label_blocks: HashMap<LabelId, BlockId>,
    loop_headers: HashMap<StatementOrigin, BlockId>,
    pending_gotos: HashMap<LabelId, Vec<BlockId>>,
    goto_bindings: Vec<&'a GotoBinding>,
    labels: Vec<&'a crate::semantic::GotoLabel>,
    goto_cursor: usize,
    label_cursor: usize,
}

impl<'a> CfgBuilder<'a> {
    /// Creates a builder for the given semantic model.
    pub fn new(model: &'a SemanticModel) -> Self {
        Self {
            model,
            function_scope: model.chunk_scope(),
            blocks: Vec::new(),
            entry: BlockId(0),
            exit: BlockId(0),
            current: None,
            loops: Vec::new(),
            label_blocks: HashMap::new(),
            loop_headers: HashMap::new(),
            pending_gotos: HashMap::new(),
            goto_bindings: Vec::new(),
            labels: Vec::new(),
            goto_cursor: 0,
            label_cursor: 0,
        }
    }

    /// Builds the control flow graph of the main chunk.
    pub fn build_chunk(mut self, block: &Block) -> Result<ControlFlowGraph, CfgError> {
        self.function_scope = self.model.chunk_scope();
        self.build(block)
    }

    /// Builds the control flow graph of a function body.
    ///
    /// `function_scope` must be the semantic function scope corresponding to the
    /// body (see [`SemanticModel::function_scope_of`]).
    pub fn build_function(
        mut self,
        block: &Block,
        function_scope: ScopeId,
    ) -> Result<ControlFlowGraph, CfgError> {
        self.function_scope = function_scope;
        self.build(block)
    }

    fn build(mut self, block: &Block) -> Result<ControlFlowGraph, CfgError> {
        self.entry = self.create_block();
        self.exit = self.create_block();
        self.current = Some(self.entry);

        self.goto_bindings = self
            .model
            .goto_bindings_in_function(self.function_scope)
            .collect();
        self.labels = self.model.labels_in_function(self.function_scope).collect();

        self.lower_block_contents(block)?;

        if self.current.is_some() {
            self.terminate(Terminator::Return(None));
        }

        if let Some((label, _)) = self.pending_gotos.iter().next() {
            return Err(CfgError {
                kind: CfgErrorKind::UnresolvedGoto {
                    name: self.model.label(*label).name.clone(),
                },
                line: self.model.label(*label).token.map(|token| token.line),
            });
        }

        if self.goto_cursor != self.goto_bindings.len() {
            let name = self.goto_bindings[self.goto_cursor].site.name.clone();
            return Err(CfgError {
                kind: CfgErrorKind::MissingSemanticRecord { kind: "goto", name },
                line: None,
            });
        }

        if self.label_cursor != self.labels.len() {
            let name = self.labels[self.label_cursor].name.clone();
            return Err(CfgError {
                kind: CfgErrorKind::MissingSemanticRecord {
                    kind: "label",
                    name,
                },
                line: None,
            });
        }

        let mut graph = ControlFlowGraph::new(self.entry, self.exit, self.function_scope);
        graph.blocks = self.blocks;
        graph.label_blocks = self.label_blocks;
        graph.loop_headers = self.loop_headers;
        Ok(graph)
    }

    // ------------------------------------------------------------------
    // Block construction helpers
    // ------------------------------------------------------------------

    fn create_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock::new(id));
        id
    }

    fn current_block(&mut self) -> BlockId {
        match self.current {
            Some(id) => id,
            None => {
                let id = self.create_block();
                self.current = Some(id);
                id
            }
        }
    }

    fn terminate(&mut self, terminator: Terminator) {
        let current = self
            .current
            .take()
            .expect("cannot terminate when no block is active");
        self.set_terminator(current, terminator, None);
    }

    fn set_terminator(
        &mut self,
        block: BlockId,
        terminator: Terminator,
        branch_origin: Option<ConditionOrigin>,
    ) {
        let graph_exit = self.exit;
        self.blocks[block.index()].branch_origin = branch_origin;
        self.blocks[block.index()].terminator = terminator.clone();
        for target in terminator.targets(graph_exit) {
            let successors = &mut self.blocks[block.index()].successors;
            if !successors.contains(&target) {
                successors.push(target);
                self.blocks[target.index()].predecessors.push(block);
            }
        }
    }

    fn emit_statement(&mut self, statement: Statement, origin: StatementOrigin) {
        let block = self.current_block();
        self.blocks[block.index()].statements.push(statement);
        self.blocks[block.index()].statement_origins.push(origin);
    }

    // ------------------------------------------------------------------
    // Lowering
    // ------------------------------------------------------------------

    fn lower_block_contents(&mut self, block: &Block) -> Result<(), CfgError> {
        let identity = block.get_identity().ok_or(CfgError {
            kind: CfgErrorKind::MissingBlockIdentity,
            line: None,
        })?;
        for (index, statement) in block.iter_statements().enumerate() {
            let origin = StatementOrigin {
                block: identity,
                index: index as u32,
            };
            self.lower_statement(statement, origin)?;
        }
        if let Some(last_statement) = block.get_last_statement() {
            self.lower_last_statement(last_statement)?;
        }
        Ok(())
    }

    fn lower_statement(
        &mut self,
        statement: &Statement,
        origin: StatementOrigin,
    ) -> Result<(), CfgError> {
        match statement {
            Statement::Assign(_)
            | Statement::CompoundAssign(_)
            | Statement::Call(_)
            | Statement::Function(_)
            | Statement::LocalAssign(_)
            | Statement::LocalFunction(_)
            | Statement::TypeDeclaration(_)
            | Statement::TypeFunction(_) => {
                self.emit_statement(statement.clone(), origin);
                Ok(())
            }
            Statement::Do(do_statement) => self.lower_block_contents(do_statement.get_block()),
            Statement::GenericFor(generic_for) => {
                let defines = self.resolve_iteration_defines(
                    generic_for
                        .iter_identifiers()
                        .map(|identifier| identifier.get_identifier()),
                )?;
                self.lower_for(
                    generic_for.iter_expressions().cloned().collect(),
                    generic_for.get_block(),
                    defines,
                    origin,
                )
            }
            Statement::Goto(goto_statement) => self.lower_goto(goto_statement),
            Statement::If(if_statement) => self.lower_if(if_statement, origin),
            Statement::Label(label_statement) => self.lower_label(label_statement),
            Statement::NumericFor(numeric_for) => {
                let defines = self
                    .resolve_iteration_defines([numeric_for.get_identifier().get_identifier()])?;
                let mut expressions = vec![
                    numeric_for.get_start().clone(),
                    numeric_for.get_end().clone(),
                ];
                if let Some(step) = numeric_for.get_step() {
                    expressions.push(step.clone());
                }
                self.lower_for(expressions, numeric_for.get_block(), defines, origin)
            }
            Statement::Repeat(repeat) => self.lower_repeat(repeat, origin),
            Statement::While(while_statement) => self.lower_while(while_statement, origin),
        }
    }

    /// Resolves the iteration variables of a `for` loop header.
    fn resolve_iteration_defines<'b>(
        &self,
        identifiers: impl IntoIterator<Item = &'b crate::nodes::Identifier>,
    ) -> Result<Vec<SymbolId>, CfgError> {
        identifiers
            .into_iter()
            .map(|identifier| {
                self.model
                    .resolve_identifier(identifier)
                    .ok_or_else(|| CfgError {
                        kind: CfgErrorKind::UnresolvedIdentifier {
                            name: identifier.get_name().clone(),
                        },
                        line: identifier
                            .get_token()
                            .and_then(|token| token.get_line_number()),
                    })
            })
            .collect()
    }

    fn lower_last_statement(&mut self, statement: &LastStatement) -> Result<(), CfgError> {
        match statement {
            LastStatement::Break(token) => {
                let context = self.loops.last().copied().ok_or_else(|| CfgError {
                    kind: CfgErrorKind::BreakOutsideLoop,
                    line: token.as_ref().and_then(|token| token.get_line_number()),
                })?;
                // A previous statement (for example a `goto`) may already have
                // terminated this block, making the `break` unreachable. The
                // loop check above still runs so an invalid `break` is
                // reported, but no second terminator is installed.
                if self.current.is_some() {
                    self.terminate(Terminator::Break(context.break_target));
                }
                Ok(())
            }
            LastStatement::Continue(token) => {
                let context = self.loops.last().copied().ok_or_else(|| CfgError {
                    kind: CfgErrorKind::ContinueOutsideLoop,
                    line: token.as_ref().and_then(|token| token.get_line_number()),
                })?;
                if self.current.is_some() {
                    self.terminate(Terminator::Continue(context.continue_target));
                }
                Ok(())
            }
            LastStatement::Return(_) => {
                if self.current.is_some() {
                    self.terminate(Terminator::Return(Some(statement.clone())));
                }
                Ok(())
            }
        }
    }

    fn lower_if(
        &mut self,
        if_statement: &IfStatement,
        origin: StatementOrigin,
    ) -> Result<(), CfgError> {
        let branches = if_statement.get_branches();

        if branches.is_empty() {
            return Ok(());
        }

        let merge = self.create_block();

        let first_condition = self.current_block();
        let mut condition_blocks = vec![first_condition];
        for _ in 1..branches.len() {
            condition_blocks.push(self.create_block());
        }

        let else_block = if if_statement.get_else_block().is_some() {
            self.create_block()
        } else {
            merge
        };

        let body_blocks: Vec<BlockId> = branches.iter().map(|_| self.create_block()).collect();

        for (index, branch) in branches.iter().enumerate() {
            let next = if index + 1 < branches.len() {
                condition_blocks[index + 1]
            } else {
                else_block
            };

            self.set_terminator(
                condition_blocks[index],
                Terminator::Branch {
                    condition: branch.get_condition().clone(),
                    then_block: body_blocks[index],
                    else_block: next,
                },
                Some(ConditionOrigin::new(
                    origin,
                    ConditionKind::IfBranch(index as u32),
                )),
            );

            self.current = Some(body_blocks[index]);
            self.lower_block_contents(branch.get_block())?;
            if self.current.is_some() {
                self.terminate(Terminator::Jump(merge));
            }
        }

        if let Some(block) = if_statement.get_else_block() {
            self.current = Some(else_block);
            self.lower_block_contents(block)?;
            if self.current.is_some() {
                self.terminate(Terminator::Jump(merge));
            }
        }

        self.current = Some(merge);
        Ok(())
    }

    fn lower_while(
        &mut self,
        while_statement: &crate::nodes::WhileStatement,
        origin: StatementOrigin,
    ) -> Result<(), CfgError> {
        let header = self.create_block();
        let body = self.create_block();
        let exit = self.create_block();

        if self.current.is_some() {
            self.terminate(Terminator::Jump(header));
        }

        self.set_terminator(
            header,
            Terminator::Branch {
                condition: while_statement.get_condition().clone(),
                then_block: body,
                else_block: exit,
            },
            Some(ConditionOrigin::new(origin, ConditionKind::While)),
        );
        self.loop_headers.insert(origin, header);

        self.loops.push(LoopContext {
            break_target: exit,
            continue_target: header,
        });
        self.current = Some(body);
        self.lower_block_contents(while_statement.get_block())?;
        if self.current.is_some() {
            self.terminate(Terminator::Jump(header));
        }
        self.loops.pop();

        self.current = Some(exit);
        Ok(())
    }

    fn lower_repeat(
        &mut self,
        repeat: &crate::nodes::RepeatStatement,
        origin: StatementOrigin,
    ) -> Result<(), CfgError> {
        let body = self.create_block();
        let condition = self.create_block();
        let exit = self.create_block();

        if self.current.is_some() {
            self.terminate(Terminator::Jump(body));
        }

        self.loops.push(LoopContext {
            break_target: exit,
            continue_target: condition,
        });
        self.current = Some(body);
        self.lower_block_contents(repeat.get_block())?;
        if self.current.is_some() {
            self.terminate(Terminator::Jump(condition));
        }
        self.loops.pop();

        self.set_terminator(
            condition,
            Terminator::Branch {
                condition: repeat.get_condition().clone(),
                then_block: exit,
                else_block: body,
            },
            Some(ConditionOrigin::new(origin, ConditionKind::Repeat)),
        );
        self.loop_headers.insert(origin, body);

        self.current = Some(exit);
        Ok(())
    }

    fn lower_for(
        &mut self,
        init_expressions: Vec<crate::nodes::Expression>,
        block: &Block,
        defines: Vec<SymbolId>,
        origin: StatementOrigin,
    ) -> Result<(), CfgError> {
        let header = self.create_block();
        let body = self.create_block();
        let exit = self.create_block();

        let init = self.current_block();
        for expression in init_expressions {
            self.blocks[init.index()].expressions.push(expression);
        }
        self.terminate(Terminator::Jump(header));

        self.set_terminator(
            header,
            Terminator::ForLoop {
                body,
                exit,
                defines,
            },
            None,
        );
        self.loop_headers.insert(origin, header);

        self.loops.push(LoopContext {
            break_target: exit,
            continue_target: header,
        });
        self.current = Some(body);
        self.lower_block_contents(block)?;
        if self.current.is_some() {
            self.terminate(Terminator::Jump(header));
        }
        self.loops.pop();

        self.current = Some(exit);
        Ok(())
    }

    fn lower_goto(&mut self, goto_statement: &GotoStatement) -> Result<(), CfgError> {
        let label = self.next_goto_label(goto_statement)?;
        let current = self.current_block();
        self.terminate(Terminator::Unreachable);

        if let Some(&target) = self.label_blocks.get(&label) {
            self.set_terminator(current, Terminator::Jump(target), None);
        } else {
            self.pending_gotos.entry(label).or_default().push(current);
        }

        Ok(())
    }

    fn lower_label(&mut self, label_statement: &LabelStatement) -> Result<(), CfgError> {
        let label = self.next_label_id(label_statement)?;

        let block = self.create_block();
        if self.current.is_some() {
            self.terminate(Terminator::Jump(block));
        }
        self.current = Some(block);

        if self.label_blocks.insert(label, block).is_some() {
            return Err(CfgError {
                kind: CfgErrorKind::SemanticMismatch {
                    kind: "label",
                    name: label_statement.get_label_name().to_string(),
                    recorded: "duplicate label block".to_string(),
                },
                line: label_statement
                    .get_label()
                    .get_token()
                    .and_then(|token| token.get_line_number()),
            });
        }

        if let Some(pending) = self.pending_gotos.remove(&label) {
            for pending_block in pending {
                self.set_terminator(pending_block, Terminator::Jump(block), None);
            }
        }

        Ok(())
    }

    fn next_goto_label(&mut self, statement: &GotoStatement) -> Result<LabelId, CfgError> {
        let name = statement.get_label_name();
        let Some(binding) = self.goto_bindings.get(self.goto_cursor) else {
            return Err(CfgError {
                kind: CfgErrorKind::MissingSemanticRecord {
                    kind: "goto",
                    name: name.to_string(),
                },
                line: line_of_goto(statement),
            });
        };

        if binding.site.name != name {
            return Err(CfgError {
                kind: CfgErrorKind::SemanticMismatch {
                    kind: "goto",
                    name: name.to_string(),
                    recorded: binding.site.name.clone(),
                },
                line: line_of_goto(statement),
            });
        }

        self.goto_cursor += 1;
        Ok(binding.label)
    }

    fn next_label_id(&mut self, statement: &LabelStatement) -> Result<LabelId, CfgError> {
        let name = statement.get_label_name();
        let Some(label) = self.labels.get(self.label_cursor) else {
            return Err(CfgError {
                kind: CfgErrorKind::MissingSemanticRecord {
                    kind: "label",
                    name: name.to_string(),
                },
                line: line_of_label(statement),
            });
        };

        if label.name != name {
            return Err(CfgError {
                kind: CfgErrorKind::SemanticMismatch {
                    kind: "label",
                    name: name.to_string(),
                    recorded: label.name.clone(),
                },
                line: line_of_label(statement),
            });
        }

        self.label_cursor += 1;
        Ok(label.id)
    }
}

fn line_of_goto(statement: &GotoStatement) -> Option<usize> {
    statement
        .get_label()
        .get_token()
        .and_then(|token| token.get_line_number())
}

fn line_of_label(statement: &LabelStatement) -> Option<usize> {
    statement
        .get_label()
        .get_token()
        .and_then(|token| token.get_line_number())
}

/// Builds the [`ControlFlowGraph`] of the main chunk.
pub fn build_chunk(block: &Block, model: &SemanticModel) -> Result<ControlFlowGraph, CfgError> {
    CfgBuilder::new(model).build_chunk(block)
}

/// Builds the [`ControlFlowGraph`] of a function body.
///
/// `function_scope` must be the semantic function scope of the body, which can
/// be obtained from [`SemanticModel::function_scope_of`].
pub fn build_function(
    block: &Block,
    model: &SemanticModel,
    function_scope: ScopeId,
) -> Result<ControlFlowGraph, CfgError> {
    CfgBuilder::new(model).build_function(block, function_scope)
}

/// Validates the loop context of the chunk and of every nested function body.
///
/// A `break`/`continue` is only valid if the *enclosing loop-bearing region* —
/// the chunk or the innermost function — has an enclosing loop. [`build_chunk`]
/// validates the chunk; a nested function body is a separate control-flow
/// region, so this also walks every function (statement, local function, type
/// function and function expression) and rebuilds its control-flow graph.
/// A function body that has no scope in `model` is a model/AST mismatch and is
/// reported as [`CfgErrorKind::MissingBlockIdentity`] rather than skipped.
pub fn validate_function_loop_context(
    block: &Block,
    model: &SemanticModel,
) -> Result<(), CfgError> {
    use crate::process::{ImmutableProcessor, ImmutableVisitor, Walk};

    build_chunk(block, model)?;

    struct Validator<'a> {
        model: &'a SemanticModel,
        error: Option<CfgError>,
    }

    impl Validator<'_> {
        fn check(&mut self, body: &Block) {
            if self.error.is_some() {
                return;
            }
            let function_scope = match self.model.try_scope_of(body) {
                Ok(scope) => scope,
                Err(_) => {
                    self.error = Some(CfgError {
                        kind: CfgErrorKind::MissingBlockIdentity,
                        line: None,
                    });
                    return;
                }
            };
            if let Err(error) = build_function(body, self.model, function_scope) {
                self.error = Some(error);
            }
        }
    }

    impl ImmutableProcessor for Validator<'_> {
        fn process_statement(&mut self, statement: &crate::nodes::Statement) {
            use crate::nodes::Statement;
            match statement {
                Statement::Function(function) => self.check(function.get_block()),
                Statement::LocalFunction(function) => self.check(function.get_block()),
                Statement::TypeFunction(function) => self.check(function.get_block()),
                _ => {}
            }
        }

        fn process_expression(&mut self, expression: &crate::nodes::Expression) {
            use crate::nodes::Expression;
            if let Expression::Function(function) = expression {
                self.check(function.get_block());
            }
        }
    }

    let mut validator = Validator { model, error: None };
    Walk::visit_block(block, &mut validator);

    match validator.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
