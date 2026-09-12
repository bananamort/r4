use crate::nodes::{Block, Statement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};

#[derive(Debug, Default)]
struct EmptyDoFilter {
    mutated: bool,
}

impl EmptyDoFilter {
    pub fn has_mutated(&self) -> bool {
        self.mutated
    }
}

impl NodeProcessor for EmptyDoFilter {
    fn process_block(&mut self, block: &mut Block) {
        block.filter_statements(|statement| match statement {
            Statement::Do(do_statement) => {
                self.mutated = do_statement.get_block().is_empty();
                !self.mutated
            }
            _ => true,
        });
    }
}

/// Removes empty `do ... end` statements, including nested ones.
pub fn remove_empty_do(block: &mut Block) {
    loop {
        let mut processor = EmptyDoFilter::default();
        DefaultVisitor::visit_block(block, &mut processor);
        if !processor.has_mutated() {
            break;
        }
    }
}
