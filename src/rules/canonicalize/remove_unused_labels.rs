use std::collections::HashSet;
use std::fmt;

use crate::nodes::{Block, LabelStatement, Statement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::{LabelId, SemanticModel};

/// An error produced while removing unused labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnusedLabelError {
    /// A label statement is not bound in the semantic model, so it cannot be
    /// removed. This usually means the AST does not match the model.
    UnresolvedLabel {
        /// The name of the label that could not be resolved.
        name: String,
    },
}

impl fmt::Display for UnusedLabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnusedLabelError::UnresolvedLabel { name } => {
                write!(f, "label `{}` is not bound in the semantic model", name)
            }
        }
    }
}

impl std::error::Error for UnusedLabelError {}

/// Removes `::label::` statements that no `goto` targets.
///
/// The semantic model resolves go-to bindings, so labels targeted only from
/// nested scopes or nested functions are recognized as used. Labels are matched
/// through their structural bindings, so the pass works on token-less trees.
pub fn remove_unused_labels(
    block: &mut Block,
    model: &SemanticModel,
) -> Result<usize, UnusedLabelError> {
    let used: HashSet<LabelId> = model
        .goto_bindings
        .iter()
        .map(|binding| binding.label)
        .collect();

    let unused: HashSet<LabelId> = model
        .labels
        .iter()
        .filter(|label| !used.contains(&label.id))
        .map(|label| label.id)
        .collect();

    if unused.is_empty() {
        return Ok(0);
    }

    validate_labels(block, model)?;

    let mut remover = LabelRemover {
        model,
        unused,
        removed: 0,
    };
    DefaultVisitor::visit_block(block, &mut remover);

    Ok(remover.removed)
}

fn validate_labels(block: &mut Block, model: &SemanticModel) -> Result<(), UnusedLabelError> {
    struct Validator<'a> {
        model: &'a SemanticModel,
        error: Option<UnusedLabelError>,
    }

    impl Validator<'_> {
        fn check(&mut self, label: &LabelStatement) {
            if self.error.is_some() {
                return;
            }
            if self.model.resolve_label(label.get_label()).is_none() {
                self.error = Some(UnusedLabelError::UnresolvedLabel {
                    name: label.get_label_name().to_string(),
                });
            }
        }
    }

    impl NodeProcessor for Validator<'_> {
        fn process_label_statement(&mut self, label: &mut LabelStatement) {
            self.check(label);
        }
    }

    let mut validator = Validator { model, error: None };
    DefaultVisitor::visit_block(block, &mut validator);

    match validator.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct LabelRemover<'a> {
    model: &'a SemanticModel,
    unused: HashSet<LabelId>,
    removed: usize,
}

impl NodeProcessor for LabelRemover<'_> {
    fn process_block(&mut self, block: &mut Block) {
        let mut statements = block.take_statements();
        statements.retain(|statement| {
            let Statement::Label(label) = statement else {
                return true;
            };
            let id = self
                .model
                .resolve_label(label.get_label())
                .expect("labels were validated before the rewrite");
            if self.unused.contains(&id) {
                self.removed += 1;
                false
            } else {
                true
            }
        });
        block.set_statements(statements);
    }
}
