use crate::nodes::{AssignmentKind, Block, FunctionAssignment, VariableAssignment};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

struct Processor;

impl NodeProcessor for Processor {
    fn process_local_assign_statement(&mut self, assign: &mut VariableAssignment) {
        assign.set_assignment_kind(AssignmentKind::Local);
    }

    fn process_local_function_statement(&mut self, function: &mut FunctionAssignment) {
        function.set_assignment_kind(AssignmentKind::Local);
    }
}

/// Converts Luau `const` declarations into `local` declarations.
///
/// This is a pure syntax lowering: the target of the declaration and the
/// scope it introduces are unchanged, only the keyword is normalized.
pub fn make_assignment_local(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor;
    DefaultVisitor::visit_block(block, &mut processor);
}
