use std::collections::HashSet;
use std::fmt;

use crate::nodes::{Block, Statement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{LabelId, SemanticModel, TokenKey};

/// An error produced while removing unused labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnusedLabelError {
    /// An unused label has no source position, so it cannot be removed.
    /// Parse with token preservation to run this pass.
    MissingTokenData {
        /// The name of the label that could not be removed.
        name: String,
    },
}

impl fmt::Display for UnusedLabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnusedLabelError::MissingTokenData { name } => write!(
                f,
                "label `{}` has no token data; parse with `preserve_tokens` to remove unused labels",
                name
            ),
        }
    }
}

impl std::error::Error for UnusedLabelError {}

/// Removes `::label::` statements that no `goto` targets.
///
/// The semantic model resolves go-to bindings, so labels targeted only from
/// nested scopes or nested functions are recognized as used. Labels without
/// token data cannot be located in the AST; the pass fails without modifying
/// the block in that case.
pub fn remove_unused_labels(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, UnusedLabelError> {
    let used: HashSet<LabelId> = model
        .goto_bindings
        .iter()
        .map(|binding| binding.label)
        .collect();

    let mut unused: HashSet<TokenKey> = HashSet::new();
    for label in &model.labels {
        if used.contains(&label.id) {
            continue;
        }
        match label.token {
            Some(token) => {
                unused.insert(token);
            }
            None => {
                return Err(UnusedLabelError::MissingTokenData {
                    name: label.name.clone(),
                })
            }
        }
    }

    if unused.is_empty() {
        return Ok(0);
    }

    let mut remover = LabelRemover { unused, removed: 0 };
    DefaultVisitor::visit_block(block, &mut remover);

    Ok(remover.removed)
}

struct LabelRemover {
    unused: HashSet<TokenKey>,
    removed: usize,
}

impl NodeProcessor for LabelRemover {
    fn process_block(&mut self, block: &mut Block) {
        let mut statements = block.take_statements();
        statements.retain(|statement| {
            let Statement::Label(label) = statement else {
                return true;
            };
            let key = label.get_label().get_token().and_then(TokenKey::of);
            if key.is_some_and(|key| self.unused.contains(&key)) {
                self.removed += 1;
                false
            } else {
                true
            }
        });
        block.set_statements(statements);
    }
}
