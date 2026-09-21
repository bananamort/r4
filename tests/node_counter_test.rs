//! Tests for the node counter visitor.

use r4::process::{DefaultVisitor, NodeCounter, NodeVisitor};
use r4::Parser;

#[test]
fn node_counter_counts_every_construct() {
    let source = concat!(
        "local x = compute(input)\n",
        "local function g() end\n",
        "function f() return 1 end\n",
        "x = 1\n",
        "x += 1\n",
        "do end\n",
        "for i = 1, 2 do end\n",
        "for k, v in pairs(t) do break end\n",
        "if a then end\n",
        "repeat until b\n",
        "while c do continue end\n",
        "return `value {x}`\n",
    );
    let mut block = Parser::default().parse(source).expect("valid code");
    let mut counter = NodeCounter::new();
    DefaultVisitor::visit_block(&mut block, &mut counter);

    assert!(counter.block_count > 1, "nested blocks are counted");
    assert_eq!(counter.function_call_count, 2, "compute() and pairs()");
    assert_eq!(counter.assign_count, 1);
    assert_eq!(counter.compound_assign, 1);
    assert_eq!(counter.do_count, 1);
    assert_eq!(counter.function_count, 1);
    assert_eq!(counter.generic_for_count, 1);
    assert_eq!(counter.if_count, 1);
    assert_eq!(counter.local_assign_count, 1);
    assert_eq!(counter.local_function_count, 1);
    assert_eq!(counter.numeric_for_count, 1);
    assert_eq!(counter.repeat_count, 1);
    assert_eq!(counter.while_count, 1);
    assert_eq!(counter.break_count, 1);
    assert_eq!(counter.continue_count, 1);
    assert_eq!(counter.return_count, 2);
    assert!(counter.expression_count > 0);
    assert!(counter.variable_count > 0);
    assert_eq!(counter.interpolated_string_count, 1);
}

#[test]
fn default_counter_is_zeroed() {
    let counter = NodeCounter::default();
    assert_eq!(counter.block_count, 0);
    assert_eq!(counter.expression_count, 0);
    assert_eq!(counter.variable_count, 0);
    assert_eq!(counter.interpolated_string_count, 0);
}
