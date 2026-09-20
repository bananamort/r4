use crate::nodes::{Block, FunctionStatement};
use crate::process::{DefaultVisitor, NodeProcessor, NodeVisitor};
use crate::semantic::SemanticModel;

struct Processor;

impl NodeProcessor for Processor {
    fn process_function_statement(&mut self, function: &mut FunctionStatement) {
        function.remove_method();
    }
}

/// Converts method function definitions into regular function definitions.
///
/// `function value:method(...)` becomes `function value.method(self, ...)`,
/// inserting the implicit `self` parameter.
pub fn remove_method_definition(block: &mut Block, _model: &SemanticModel) {
    let mut processor = Processor;
    DefaultVisitor::visit_block(block, &mut processor);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::test_support::{analyze, generate};

    #[test]
    fn rewrites_method_definitions_with_an_explicit_self() {
        let (mut block, model) = analyze("function a:b(x) return self end");

        remove_method_definition(&mut block, &model);

        let output = generate(&block);
        assert!(
            output.contains("function a.b(self, x)"),
            "the implicit self must become a parameter: {output}"
        );
        assert!(output.contains("return self"), "unexpected: {output}");
    }
}
