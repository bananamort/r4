use crate::nodes::{Expression, LastStatement, Statement};
use crate::semantic::SymbolId;

use super::provenance::{ConditionOrigin, StatementOrigin};

/// Identifies a basic block inside a [`ControlFlowGraph`](super::ControlFlowGraph).
///
/// The identifier is an index into the graph's flat `blocks` arena.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

impl BlockId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The control flow instruction that ends a [`BasicBlock`].
#[derive(Clone, Debug, PartialEq)]
pub enum Terminator {
    /// Unconditional jump to a target block.
    Jump(BlockId),
    /// Conditional branch: the `then_block` is taken when the condition is
    /// truthy, the `else_block` otherwise.
    Branch {
        /// The evaluated condition.
        condition: Expression,
        /// Target when the condition is truthy.
        then_block: BlockId,
        /// Target when the condition is falsy.
        else_block: BlockId,
    },
    /// Implicit iteration test of a `for` loop. The `body` is executed while the
    /// loop is not exhausted, otherwise control continues at `exit`. `defines`
    /// are the iteration variables the header introduces on every iteration.
    ForLoop {
        /// The loop body.
        body: BlockId,
        /// The block reached when the loop ends.
        exit: BlockId,
        /// Iteration variables defined by the loop header.
        defines: Vec<SymbolId>,
    },
    /// Return from the current function (or chunk). The return values, if any,
    /// are preserved for data-flow analysis.
    Return(Option<LastStatement>),
    /// Explicit `break` out of a loop, jumping to the loop exit block.
    Break(BlockId),
    /// Explicit `continue`, jumping to the loop continuation point.
    Continue(BlockId),
    /// Control does not continue (dead code or an unresolved placeholder).
    Unreachable,
}

impl Terminator {
    /// Returns the successor blocks of this terminator, given the graph exit
    /// block (used by `Return` terminators).
    pub fn targets(&self, exit: BlockId) -> Vec<BlockId> {
        match self {
            Terminator::Jump(target) | Terminator::Break(target) | Terminator::Continue(target) => {
                vec![*target]
            }
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => vec![*then_block, *else_block],
            Terminator::ForLoop { body, exit, .. } => vec![*body, *exit],
            Terminator::Return(_) => vec![exit],
            Terminator::Unreachable => Vec::new(),
        }
    }

    /// Returns whether this terminator ends a reachable path (everything but
    /// `Unreachable`).
    #[inline]
    pub fn is_live(&self) -> bool {
        !matches!(self, Terminator::Unreachable)
    }
}

/// A maximal sequence of statements with a single entry and a single
/// [`Terminator`].
#[derive(Clone, Debug, PartialEq)]
pub struct BasicBlock {
    /// The identifier of this block.
    pub id: BlockId,
    /// Statements executed in order when entering this block.
    pub statements: Vec<Statement>,
    /// The structural origin of each statement, parallel to `statements`.
    pub statement_origins: Vec<StatementOrigin>,
    /// Expressions evaluated after the statements and before the terminator
    /// (used for `for` loop initialization expressions).
    pub expressions: Vec<Expression>,
    /// The AST construct that produced a [`Terminator::Branch`], when the
    /// terminator is a branch lowered from source.
    pub branch_origin: Option<ConditionOrigin>,
    /// The structural position of the source last statement that produced the
    /// terminator, when it was lowered from a `return`, `break` or `continue`.
    pub terminator_origin: Option<StatementOrigin>,
    /// Blocks that can transfer control to this block.
    pub predecessors: Vec<BlockId>,
    /// Blocks this block can transfer control to.
    pub successors: Vec<BlockId>,
    /// The instruction that ends this block.
    pub terminator: Terminator,
}

impl BasicBlock {
    /// Creates a new, empty block that is not terminated yet.
    pub(crate) fn new(id: BlockId) -> Self {
        Self {
            id,
            statements: Vec::new(),
            statement_origins: Vec::new(),
            expressions: Vec::new(),
            branch_origin: None,
            terminator_origin: None,
            predecessors: Vec::new(),
            successors: Vec::new(),
            terminator: Terminator::Unreachable,
        }
    }

    /// Returns whether this block has no executable content.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty() && self.expressions.is_empty()
    }

    /// Returns the number of statements in this block.
    #[inline]
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// Returns the structural origin of the statement at `index`, if it was
    /// lowered from the AST.
    #[inline]
    pub fn statement_origin(&self, index: usize) -> Option<StatementOrigin> {
        self.statement_origins.get(index).copied()
    }
}
