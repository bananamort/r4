use r4::cfg::analysis::{
    fold_constant_branches, fold_constant_branches_with_propagation, ConstantPropagation,
    DominatorTree, Liveness, Reachability,
};
use r4::cfg::{
    build_chunk, build_function, validate_block_identities, BlockId, ConditionKind,
    ConditionOrigin, ControlFlowGraph, StatementOrigin, Terminator,
};
use r4::nodes::{Block, Prefix, Statement};
use r4::process::LuaValue;
use r4::semantic::{ScopeKind, SemanticAnalyzer, SemanticModel, SymbolId};
use r4::Parser;

fn build(code: &str) -> (ControlFlowGraph, SemanticModel) {
    let block = Parser::default()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"));
    let cfg = build_chunk(&block, &model)
        .unwrap_or_else(|error| panic!("cfg construction failed for `{code}`: {error}"));

    cfg.verify_invariants()
        .unwrap_or_else(|violations| panic!("invariant violations for `{code}`: {violations:?}"));

    (cfg, model)
}

fn build_lua52(code: &str) -> (ControlFlowGraph, SemanticModel) {
    let block = Parser::default()
        .lua52()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"));
    let cfg = build_chunk(&block, &model)
        .unwrap_or_else(|error| panic!("cfg construction failed for `{code}`: {error}"));

    cfg.verify_invariants()
        .unwrap_or_else(|violations| panic!("invariant violations for `{code}`: {violations:?}"));

    (cfg, model)
}

fn build_with_tokens(code: &str) -> (ControlFlowGraph, SemanticModel) {
    let block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .unwrap_or_else(|error| panic!("failed to parse `{code}`: {error}"));
    let model = SemanticAnalyzer::new()
        .analyze(&block)
        .unwrap_or_else(|error| panic!("semantic analysis failed for `{code}`: {error}"));
    let cfg = build_chunk(&block, &model)
        .unwrap_or_else(|error| panic!("cfg construction failed for `{code}`: {error}"));

    cfg.verify_invariants()
        .unwrap_or_else(|violations| panic!("invariant violations for `{code}`: {violations:?}"));

    (cfg, model)
}

fn assert_invariants(cfg: &ControlFlowGraph) {
    if let Err(violations) = cfg.verify_invariants() {
        panic!("CFG invariant violations: {violations:?}");
    }
}

fn branch_blocks(cfg: &ControlFlowGraph, block: BlockId) -> (BlockId, BlockId) {
    match cfg.terminator(block) {
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => (*then_block, *else_block),
        other => panic!("block {block:?} is not a branch: {other:?}"),
    }
}

fn jump_target(cfg: &ControlFlowGraph, block: BlockId) -> BlockId {
    match cfg.terminator(block) {
        Terminator::Jump(target) => *target,
        other => panic!("block {block:?} is not a jump: {other:?}"),
    }
}

fn block_with_call(cfg: &ControlFlowGraph, name: &str) -> BlockId {
    cfg.iter_block_ids()
        .find(|block| {
            cfg.block(*block).statements.iter().any(|statement| {
                matches!(
                    statement,
                    Statement::Call(call)
                        if matches!(call.get_prefix(), Prefix::Identifier(identifier) if identifier.get_name() == name)
                )
            })
        })
        .unwrap_or_else(|| panic!("no block contains a call to `{name}`"))
}

fn symbol_named(model: &SemanticModel, name: &str) -> SymbolId {
    model
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("no symbol named `{name}`"))
        .id
}

#[test]
fn linear_code_produces_a_single_block_with_return() {
    let (cfg, _) = build(
        r#"
        local x = 1
        x = x + 1
        return x
        "#,
    );

    let entry = cfg.entry;
    assert_eq!(cfg.block(entry).statements.len(), 2);
    assert!(matches!(cfg.terminator(entry), Terminator::Return(Some(_))));
    assert_eq!(cfg.successors(entry), &[cfg.exit]);
    assert!(cfg.successors(cfg.exit).is_empty());

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_reachable(entry));
    assert!(reachability.is_reachable(cfg.exit));
    assert_eq!(reachability.unreachable_blocks().count(), 0);
}

#[test]
fn if_else_branches_converge_at_merge() {
    let (cfg, _) = build(
        r#"
        if condition then
            f()
        else
            g()
        end
        h()
        "#,
    );

    let (then_block, else_block) = branch_blocks(&cfg, cfg.entry);
    assert_ne!(then_block, else_block);

    let then_merge = jump_target(&cfg, then_block);
    let else_merge = jump_target(&cfg, else_block);
    assert_eq!(then_merge, else_merge, "branches must converge");

    let merge = then_merge;
    assert!(cfg.predecessors(merge).contains(&then_block));
    assert!(cfg.predecessors(merge).contains(&else_block));

    assert!(
        cfg.iter_block_ids().any(|block| block == merge
            && cfg
                .block(block)
                .statements
                .iter()
                .any(|statement| matches!(statement, Statement::Call(_)))),
        "the merge block must contain the call to `h`"
    );

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.reachable_blocks().count() == cfg.block_count());
}

#[test]
fn if_without_else_branches_directly_to_merge() {
    let (cfg, _) = build(
        r#"
        if condition then
            f()
        end
        g()
        "#,
    );

    let (then_block, else_block) = branch_blocks(&cfg, cfg.entry);
    assert_eq!(jump_target(&cfg, then_block), else_block);
}

#[test]
fn while_loop_has_back_edge_and_break_targets_exit() {
    let (cfg, _) = build(
        r#"
        while condition do
            work()
            break
        end
        "#,
    );

    let header = jump_target(&cfg, cfg.entry);
    let (body, exit) = branch_blocks(&cfg, header);

    assert!(
        matches!(cfg.terminator(body), Terminator::Break(target) if *target == exit),
        "the `break` must target the loop exit"
    );

    // The loop condition branches to the body or the exit.
    assert_eq!(cfg.predecessors(body).to_vec(), vec![header]);
    assert!(cfg.predecessors(exit).contains(&header));
    assert!(cfg.predecessors(exit).contains(&body));
}

#[test]
fn while_loop_body_jumps_back_to_header() {
    let (cfg, _) = build(
        r#"
        while condition do
            work()
        end
        "#,
    );

    let header = jump_target(&cfg, cfg.entry);
    let (body, exit) = branch_blocks(&cfg, header);

    assert_eq!(jump_target(&cfg, body), header, "the back edge must exist");
    assert!(!cfg.predecessors(header).is_empty());
    assert!(cfg.predecessors(exit).contains(&header));
}

#[test]
fn continue_jumps_back_to_the_loop_header() {
    let (cfg, _) = build(
        r#"
        while condition do
            if skip then
                continue
            end
            work()
        end
        "#,
    );

    let header = jump_target(&cfg, cfg.entry);
    let (branch, _) = branch_blocks(&cfg, header);

    let continue_block = cfg
        .iter_block_ids()
        .find(|block| matches!(cfg.terminator(*block), Terminator::Continue(_)))
        .expect("a continue terminator must exist");

    match cfg.terminator(continue_block) {
        Terminator::Continue(target) => assert_eq!(*target, header),
        other => panic!("unexpected terminator: {other:?}"),
    }
    assert!(cfg.predecessors(branch).contains(&header));
}

#[test]
fn repeat_loop_condition_branches_back_to_body() {
    let (cfg, _) = build(
        r#"
        repeat
            work()
        until done
        "#,
    );

    let body = jump_target(&cfg, cfg.entry);
    let condition = jump_target(&cfg, body);
    let (exit, repeat_body) = branch_blocks(&cfg, condition);

    assert_eq!(
        repeat_body, body,
        "the condition must loop back to the body when it is falsy"
    );
    assert_ne!(exit, body, "the truthy branch exits the loop");
}

#[test]
fn numeric_for_uses_an_implicit_loop_test() {
    let (cfg, _) = build(
        r#"
        for i = 1, 10 do
            work()
        end
        "#,
    );

    let header = jump_target(&cfg, cfg.entry);
    let (body, exit) = match cfg.terminator(header) {
        Terminator::ForLoop { body, exit, .. } => (*body, *exit),
        other => panic!("expected a for-loop terminator, got {other:?}"),
    };

    assert_eq!(jump_target(&cfg, body), header, "the back edge must exist");
    assert!(cfg.predecessors(exit).contains(&header));
    assert_eq!(cfg.block(header).expressions.len(), 0);
    assert_eq!(
        cfg.block(cfg.entry).expressions.len(),
        2,
        "start and end expressions are evaluated in the entry block"
    );
}

#[test]
fn dead_code_after_return_is_unreachable() {
    let (cfg, _) = build(
        r#"
        do
            return 1
        end
        print("dead")
        "#,
    );

    let reachability = Reachability::analyze(&cfg);
    let dead = block_with_call(&cfg, "print");

    assert!(reachability.is_unreachable(dead));
    assert!(reachability.unreachable_blocks().count() >= 1);

    // The return block itself stays reachable.
    assert!(matches!(
        cfg.terminator(cfg.entry),
        Terminator::Return(Some(_))
    ));
    assert!(reachability.is_reachable(cfg.entry));
}

#[test]
fn goto_lowers_to_a_jump_and_skips_code() {
    let (cfg, _) = build_lua52(
        r#"
        do
            goto done
        end
        print("skipped")
        ::done::
        return
        "#,
    );

    assert_eq!(cfg.label_blocks.len(), 1);

    let label_block = *cfg.label_blocks.values().next().unwrap();
    let goto_source = cfg
        .iter_block_ids()
        .find(|block| matches!(cfg.terminator(*block), Terminator::Jump(target) if *target == label_block))
        .expect("the goto must lower to a jump to the label block");

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_reachable(goto_source));
    assert!(reachability.is_reachable(label_block));
    assert!(
        reachability.is_unreachable(block_with_call(&cfg, "print")),
        "the skipped print is dead code"
    );
}

#[test]
fn complex_program_satisfies_the_cfg_invariants() {
    let (cfg, _) = build(
        r#"
        local total = 0
        for i = 1, 10 do
            if i % 2 == 0 then
                continue
            end
            total = total + i
        end
        local n = 0
        while n < 10 do
            n = n + 1
            if n == 5 then
                break
            end
        end
        repeat
            n = n - 1
        until n == 0
        if total > 0 then
            print(total)
        else
            print(n)
        end
        return total
        "#,
    );

    assert_invariants(&cfg);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_reachable(cfg.entry));
    assert_eq!(reachability.unreachable_blocks().count(), 0);
}

#[test]
fn liveness_tracks_variables_across_branches() {
    let (cfg, model) = build_with_tokens(
        r#"
        local x = 1
        if condition then
            print(x)
        end
        "#,
    );

    let liveness = Liveness::analyze(&cfg, &model).expect("liveness must succeed with tokens");
    let x = symbol_named(&model, "x");
    let (then_block, _) = branch_blocks(&cfg, cfg.entry);

    assert!(liveness.is_live_out(cfg.entry, x));
    assert!(liveness.is_live_in(then_block, x));
}

#[test]
fn liveness_kills_reassigned_variables() {
    let (cfg, model) = build_with_tokens(
        r#"
        local x = 1
        if condition then
            x = 2
        else
            print(x)
        end
        print(x)
        "#,
    );

    let liveness = Liveness::analyze(&cfg, &model).expect("liveness must succeed with tokens");
    let x = symbol_named(&model, "x");
    let (then_block, else_block) = branch_blocks(&cfg, cfg.entry);

    assert!(
        !liveness.is_live_in(then_block, x),
        "x is redefined in the then-branch before any read"
    );
    assert!(
        liveness.is_live_in(else_block, x),
        "x is read in the else-branch"
    );

    let live_in: Vec<_> = liveness.live_in_symbols(then_block).collect();
    assert!(!live_in.contains(&x));
}

#[test]
fn liveness_reports_upward_exposed_uses() {
    let (cfg, model) = build_with_tokens(
        r#"
        local a = 1
        local b = a + c
        return b
        "#,
    );

    let liveness = Liveness::analyze(&cfg, &model).expect("liveness must succeed with tokens");
    let a = symbol_named(&model, "a");
    let c = symbol_named(&model, "c");

    let entry = cfg.entry;
    assert!(liveness.uses[entry.index()][c.index()], "c is a use");
    assert!(
        !liveness.uses[entry.index()][a.index()],
        "a is defined before it is read"
    );
    assert!(liveness.is_live_in(entry, c));
}

#[test]
fn liveness_works_without_token_data() {
    let block = Parser::default()
        .parse("local x = 1\nreturn x")
        .expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let cfg = build_chunk(&block, &model).expect("cfg");

    let liveness =
        Liveness::analyze(&cfg, &model).expect("token-less liveness analysis must succeed");
    let x = symbol_named(&model, "x");

    assert!(
        liveness.terminator_uses[cfg.entry.index()][x.index()],
        "the return statement reads x"
    );
}

#[test]
fn dominator_tree_identifies_merge_dominators() {
    let (cfg, _) = build(
        r#"
        if condition then
            f()
        else
            g()
        end
        h()
        "#,
    );

    let (then_block, else_block) = branch_blocks(&cfg, cfg.entry);
    let merge = jump_target(&cfg, then_block);
    let tree = DominatorTree::analyze(&cfg);

    assert!(tree.dominates(cfg.entry, merge));
    assert!(!tree.strictly_dominates(then_block, merge));
    assert!(!tree.strictly_dominates(else_block, merge));
    assert_eq!(tree.immediate_dominator(merge), Some(cfg.entry));
    assert!(tree.dominates(merge, merge));
}

#[test]
fn dominators_follow_loops() {
    let (cfg, _) = build(
        r#"
        while condition do
            work()
        end
        "#,
    );

    let header = jump_target(&cfg, cfg.entry);
    let (body, exit) = branch_blocks(&cfg, header);
    let tree = DominatorTree::analyze(&cfg);

    assert!(tree.dominates(cfg.entry, header));
    assert!(tree.dominates(header, body));
    assert!(tree.dominates(header, exit));
    assert_eq!(tree.immediate_dominator(body), Some(header));
    assert_eq!(tree.immediate_dominator(exit), Some(header));
}

#[test]
fn goto_can_create_a_back_edge() {
    let (cfg, _) = build_lua52(
        r#"
        local n = 0
        ::top::
        n = n + 1
        if n < 10 then
            goto top
        end
        return n
        "#,
    );

    assert_invariants(&cfg);

    let label_block = *cfg.label_blocks.values().next().expect("the label exists");
    let back_edge = cfg
        .iter_block_ids()
        .find(|block| matches!(cfg.terminator(*block), Terminator::Jump(target) if *target == label_block))
        .expect("the goto must lower to a jump to the label");
    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_reachable(back_edge));
    assert!(reachability.is_reachable(label_block));
}

#[test]
fn self_loop_goto_keeps_the_exit_unreachable() {
    let (cfg, _) = build_lua52(
        r#"
        ::top::
        goto top
        "#,
    );

    assert_invariants(&cfg);
    assert_eq!(cfg.label_blocks.len(), 1);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_reachable(cfg.entry));
    assert!(
        reachability.is_unreachable(cfg.exit),
        "an infinite goto loop never reaches the exit"
    );
}

#[test]
fn break_outside_a_loop_is_rejected() {
    let block = Parser::default().parse("break").expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

    let error = build_chunk(&block, &model).expect_err("break outside a loop must fail");
    assert!(matches!(
        error.kind,
        r4::cfg::CfgErrorKind::BreakOutsideLoop
    ));
}

#[test]
fn continue_outside_a_loop_is_rejected() {
    let block = Parser::default().parse("continue").expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

    let error = build_chunk(&block, &model).expect_err("continue outside a loop must fail");
    assert!(matches!(
        error.kind,
        r4::cfg::CfgErrorKind::ContinueOutsideLoop
    ));
}

#[test]
fn constant_branches_are_folded() {
    let (mut cfg, _) = build(
        r#"
        while false do
            work()
        end
        after()
        "#,
    );

    let folded = fold_constant_branches(&mut cfg);
    assert_eq!(folded, 1);
    assert_invariants(&cfg);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_unreachable(block_with_call(&cfg, "work")));
    assert!(reachability.is_reachable(block_with_call(&cfg, "after")));
}

#[test]
fn constant_if_folds_the_taken_branch_only() {
    let (mut cfg, _) = build("if false then a() else b() end");

    assert_eq!(fold_constant_branches(&mut cfg), 1);
    assert_invariants(&cfg);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_unreachable(block_with_call(&cfg, "a")));
    assert!(reachability.is_reachable(block_with_call(&cfg, "b")));
}

#[test]
fn infinite_loops_expose_unreachable_successors() {
    let (mut cfg, _) = build("while true do work() end\nafter()");

    assert_eq!(fold_constant_branches(&mut cfg), 1);
    assert_invariants(&cfg);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_reachable(block_with_call(&cfg, "work")));
    assert!(reachability.is_unreachable(block_with_call(&cfg, "after")));
}

#[test]
fn constant_conditions_through_locals_are_folded() {
    let (mut cfg, model) = build_with_tokens(
        r#"
        local enabled = false
        if enabled then
            a()
        else
            b()
        end
        "#,
    );

    let folded = fold_constant_branches_with_propagation(&mut cfg, &model)
        .expect("constant propagation must succeed with tokens");

    assert_eq!(folded, 1);
    assert_invariants(&cfg);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_unreachable(block_with_call(&cfg, "a")));
    assert!(reachability.is_reachable(block_with_call(&cfg, "b")));
}

#[test]
fn constants_from_arithmetic_are_folded() {
    let (mut cfg, model) = build_with_tokens(
        r#"
        local x = 1
        local y = x + 1
        if x + y == 3 then
            a()
        else
            b()
        end
        "#,
    );

    let folded = fold_constant_branches_with_propagation(&mut cfg, &model)
        .expect("constant propagation must succeed with tokens");

    assert_eq!(folded, 1);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_reachable(block_with_call(&cfg, "a")));
    assert!(reachability.is_unreachable(block_with_call(&cfg, "b")));
}

#[test]
fn loop_conditions_through_locals_are_folded() {
    let (mut cfg, model) = build_with_tokens(
        r#"
        local running = false
        while running do
            work()
        end
        after()
        "#,
    );

    let folded = fold_constant_branches_with_propagation(&mut cfg, &model)
        .expect("constant propagation must succeed with tokens");

    assert_eq!(folded, 1);
    assert_invariants(&cfg);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_unreachable(block_with_call(&cfg, "work")));
    assert!(reachability.is_reachable(block_with_call(&cfg, "after")));
}

#[test]
fn identical_constants_across_branches_are_folded() {
    let (mut cfg, model) = build_with_tokens(
        r#"
        local flag = false
        if condition then
            flag = false
        else
            flag = false
        end
        if flag then
            a()
        end
        "#,
    );

    let folded = fold_constant_branches_with_propagation(&mut cfg, &model)
        .expect("constant propagation must succeed with tokens");

    assert_eq!(folded, 1, "only the second condition is constant");
    assert_invariants(&cfg);

    let reachability = Reachability::analyze(&cfg);
    assert!(reachability.is_unreachable(block_with_call(&cfg, "a")));
}

#[test]
fn conflicting_constants_do_not_fold() {
    let (mut cfg, model) = build_with_tokens(
        r#"
        local flag = true
        if condition then
            flag = false
        end
        if flag then
            a()
        end
        "#,
    );

    let folded = fold_constant_branches_with_propagation(&mut cfg, &model)
        .expect("constant propagation must succeed with tokens");

    assert_eq!(folded, 0);
    assert_invariants(&cfg);
}

#[test]
fn captured_locals_are_not_propagated() {
    let (mut cfg, model) = build_with_tokens(
        r#"
        local flag = false
        local function toggle()
            flag = true
        end
        if flag then
            a()
        end
        "#,
    );

    let folded = fold_constant_branches_with_propagation(&mut cfg, &model)
        .expect("constant propagation must succeed with tokens");

    assert_eq!(folded, 0, "captured locals can change at any call");
}

#[test]
fn constant_propagation_exposes_entry_and_exit_constants() {
    let (cfg, model) = build_with_tokens("local x = 1\nlocal y = x");

    let analysis = ConstantPropagation::analyze(&cfg, &model)
        .expect("constant propagation must succeed with tokens");
    let x = symbol_named(&model, "x");
    let y = symbol_named(&model, "y");

    assert_eq!(
        analysis.block_exit_constants(cfg.entry).get(&x),
        Some(&LuaValue::Number(1.0))
    );
    assert_eq!(
        analysis.block_exit_constants(cfg.entry).get(&y),
        Some(&LuaValue::Number(1.0)),
        "initializers are evaluated against the current constants"
    );
}

#[test]
fn constant_propagation_works_without_token_data() {
    let block = Parser::default()
        .parse("local x = 1\nreturn x")
        .expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let cfg = build_chunk(&block, &model).expect("cfg");

    let analysis = ConstantPropagation::analyze(&cfg, &model)
        .expect("token-less constant propagation must succeed");
    let x = symbol_named(&model, "x");

    assert_eq!(
        analysis.block_exit_constants(cfg.entry).get(&x),
        Some(&LuaValue::Number(1.0))
    );
}

#[test]
fn non_constant_branches_are_kept() {
    let (mut cfg, _) = build("if condition then a() else b() end");

    assert_eq!(fold_constant_branches(&mut cfg), 0);
    assert_invariants(&cfg);
}

#[test]
fn function_bodies_build_their_own_graph() {
    let block = Parser::default()
        .parse(
            r#"
            local function f()
                if condition then
                    return 1
                end
                return 2
            end
            return f
            "#,
        )
        .expect("parse");

    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let chunk = build_chunk(&block, &model).expect("chunk cfg");

    let function_scope = model
        .scopes
        .iter()
        .find(|scope| scope.kind == ScopeKind::Function)
        .expect("a function scope must exist")
        .id;

    let Statement::LocalFunction(function) = block
        .first_statement()
        .expect("the function declaration is first")
    else {
        panic!("expected a local function statement");
    };

    let function_cfg =
        build_function(function.get_block(), &model, function_scope).expect("function cfg");

    assert_eq!(function_cfg.function_scope, function_scope);
    assert_eq!(chunk.function_scope, model.chunk_scope());
    assert_invariants(&function_cfg);

    // The function body starts with a conditional branch and returns.
    assert!(matches!(
        function_cfg.terminator(function_cfg.entry),
        Terminator::Branch { .. }
    ));
    assert!(function_cfg
        .iter_block_ids()
        .any(|block| matches!(function_cfg.terminator(block), Terminator::Return(_))));
}

#[test]
fn dominator_tree_detects_natural_while_loops() {
    let (cfg, _) = build("while condition do work() end");
    let tree = DominatorTree::analyze(&cfg);

    let header = jump_target(&cfg, cfg.entry);
    let (body, exit) = branch_blocks(&cfg, header);

    assert!(tree.is_loop_header(header));
    assert_eq!(tree.back_edges().len(), 1);
    assert_eq!(tree.back_edges()[0].latch, body);
    assert_eq!(tree.back_edges()[0].header, header);

    let blocks = tree
        .try_natural_loop_blocks(header, body)
        .expect("a natural loop");
    assert!(blocks.contains(&header));
    assert!(blocks.contains(&body));
    assert!(!blocks.contains(&cfg.entry));
    assert!(!blocks.contains(&exit));
    assert_eq!(tree.try_natural_loop_blocks(cfg.entry, cfg.exit), None);
}

#[test]
fn dominator_tree_detects_repeat_loops() {
    let (cfg, _) = build("repeat work() until done");
    let tree = DominatorTree::analyze(&cfg);

    let body = jump_target(&cfg, cfg.entry);
    let condition = jump_target(&cfg, body);

    assert!(tree.is_loop_header(body));
    let blocks = tree
        .try_natural_loop_blocks(body, condition)
        .expect("a natural loop");
    assert!(blocks.contains(&body));
    assert!(blocks.contains(&condition));
    assert_eq!(blocks.len(), 2);
}

#[test]
fn dominator_tree_detects_for_loops() {
    let (cfg, _) = build("for i = 1, 10 do work() end");
    let tree = DominatorTree::analyze(&cfg);

    let header = jump_target(&cfg, cfg.entry);
    let (body, exit) = match cfg.terminator(header) {
        Terminator::ForLoop { body, exit, .. } => (*body, *exit),
        other => panic!("expected a for-loop terminator, got {other:?}"),
    };

    assert!(tree.is_loop_header(header));
    let blocks = tree
        .try_natural_loop_blocks(header, body)
        .expect("a natural loop");
    assert!(blocks.contains(&header));
    assert!(blocks.contains(&body));
    assert!(!blocks.contains(&exit));
}

#[test]
fn natural_loop_blocks_returns_none_for_non_back_edges() {
    let (cfg, _) = build("return 1");
    let tree = DominatorTree::analyze(&cfg);

    assert_eq!(tree.try_natural_loop_blocks(cfg.entry, cfg.exit), None);
}

#[test]
fn dominance_prunes_after_a_removed_branch_edge() {
    let (cfg, _) = build("if false then a() else b() end\nc()");
    let tree = DominatorTree::analyze(&cfg);

    let (then_block, else_block) = branch_blocks(&cfg, cfg.entry);
    let pruned = tree.prune_after_removed_edge(&cfg, cfg.entry, then_block);

    assert!(pruned.contains(&then_block));
    assert!(!pruned.contains(&else_block));
    for block in tree.dominator_subtree(then_block) {
        assert!(
            pruned.contains(&block),
            "{block:?} is dominated by the removed target"
        );
    }
}

#[test]
fn dominance_pruning_closes_over_the_dominator_subtree() {
    let (cfg, _) = build(
        r#"
        if false then
            while condition do
                work()
            end
        end
        after()
        "#,
    );
    let tree = DominatorTree::analyze(&cfg);

    let (then_block, _) = branch_blocks(&cfg, cfg.entry);
    let pruned = tree.prune_after_removed_edge(&cfg, cfg.entry, then_block);

    for block in tree.dominator_subtree(then_block) {
        assert!(
            pruned.contains(&block),
            "{block:?} must be pruned through dominance"
        );
    }
}

#[test]
fn dominance_pruning_is_empty_when_another_path_can_reach_the_target() {
    let (cfg, _) = build_lua52(
        r#"
        do
            goto done
        end
        print("skipped")
        ::done::
        "#,
    );
    let tree = DominatorTree::analyze(&cfg);

    let label_block = *cfg
        .label_blocks
        .values()
        .next()
        .expect("the label block must exist");
    let pruned = tree.prune_after_removed_edge(&cfg, cfg.entry, label_block);
    assert!(
        pruned.is_empty(),
        "the label is also the target of the print block's edge"
    );
}

#[test]
fn unreachable_blocks_have_no_natural_loops() {
    let (cfg, _) = build("do return 1 end\nwhile condition do work() end");
    let tree = DominatorTree::analyze(&cfg);

    let reachability = Reachability::analyze(&cfg);
    for edge in tree.back_edges() {
        assert!(
            reachability.is_reachable(edge.header),
            "back edges only exist between reachable blocks"
        );
    }
}

#[test]
fn dominance_proves_unreachability_from_marked_blocks() {
    let (cfg, _) = build("do return 1 end\nafter()");
    let tree = DominatorTree::analyze(&cfg);

    let dead = block_with_call(&cfg, "after");
    assert!(tree.is_provably_unreachable(dead, |block| block == dead));
    assert!(!tree.is_provably_unreachable(cfg.entry, |block| block == dead));
}

#[test]
fn unreachable_blocks_are_not_dominated_by_entry() {
    let (cfg, _) = build(
        r#"
        do
            return 1
        end
        print("dead")
        "#,
    );

    let dead = block_with_call(&cfg, "print");
    let tree = DominatorTree::analyze(&cfg);

    assert!(!tree.dominates(cfg.entry, dead));
    assert_eq!(tree.immediate_dominator(dead), None);
}

#[test]
fn cfg_statements_reference_their_ast_blocks() {
    let block = Parser::default()
        .parse("local x = 1\nx = 2")
        .expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");

    let identity = block
        .get_identity()
        .expect("the chunk block has an identity");
    assert_eq!(
        identity.index(),
        0,
        "identities are assigned in source order"
    );

    let cfg = build_chunk(&block, &model).expect("cfg");
    let entry = cfg.block(cfg.entry);

    assert_eq!(
        entry.statement_origin(0),
        Some(StatementOrigin {
            block: identity,
            index: 0
        })
    );
    assert_eq!(
        entry.statement_origin(1),
        Some(StatementOrigin {
            block: identity,
            index: 1
        })
    );
    assert_eq!(entry.statement_origin(2), None);
}

#[test]
fn cfg_branch_terminators_reference_their_ast_statements() {
    let block = Parser::default()
        .parse("if a then b() elseif c then d() else e() end")
        .expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let identity = block
        .get_identity()
        .expect("the chunk block has an identity");
    let cfg = build_chunk(&block, &model).expect("cfg");

    let origins: Vec<Option<ConditionOrigin>> = cfg
        .blocks
        .iter()
        .filter_map(|block| {
            matches!(block.terminator, Terminator::Branch { .. }).then_some(block.branch_origin)
        })
        .collect();

    assert_eq!(
        origins,
        vec![
            Some(ConditionOrigin {
                block: identity,
                statement: 0,
                kind: ConditionKind::IfBranch(0)
            }),
            Some(ConditionOrigin {
                block: identity,
                statement: 0,
                kind: ConditionKind::IfBranch(1)
            }),
        ]
    );
}

#[test]
fn cfg_builder_rejects_a_foreign_model() {
    let block = Parser::default()
        .parse("local x = 1")
        .expect("the block must parse");
    let other = Parser::default()
        .parse("local y = 1")
        .expect("the other block must parse");
    let model = SemanticAnalyzer::new()
        .analyze(&other)
        .expect("the other block must analyze");

    let error = build_chunk(&block, &model).expect_err("a foreign model must be rejected");
    assert!(matches!(
        error.kind,
        r4::cfg::CfgErrorKind::MissingBlockIdentity
    ));
}

#[test]
fn validate_block_identities_rejects_unanalyzed_blocks() {
    let mut block: Block = Statement::Do(Default::default()).into();

    assert!(validate_block_identities(&mut block).is_err());

    let analyzed = Parser::default()
        .parse("do end")
        .expect("the block must parse");
    let _model = SemanticAnalyzer::new()
        .analyze(&analyzed)
        .expect("analysis");
    let mut analyzed = analyzed;
    assert!(validate_block_identities(&mut analyzed).is_ok());
}

#[test]
fn constant_propagation_distinguishes_same_scope_shadowing() {
    let block = Parser::default()
        .parse("local x = true\nprint(x)\nlocal x = false\nif x then dead() end")
        .expect("parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let mut cfg = build_chunk(&block, &model).expect("cfg");

    let folded = fold_constant_branches_with_propagation(&mut cfg, &model)
        .expect("propagation must succeed");

    assert_eq!(
        folded, 1,
        "the second x shadows the first and is constantly false"
    );
    assert_invariants(&cfg);
}

#[test]
fn goto_followed_by_unreachable_return_builds() {
    let block = Parser::default()
        .lua52()
        .parse("::label::\ngoto label\nreturn")
        .expect("the code must parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let cfg = build_chunk(&block, &model).expect("cfg construction must not panic");
    assert_invariants(&cfg);
}

#[test]
fn unreachable_break_outside_loop_is_still_rejected() {
    let block = Parser::default()
        .lua52()
        .parse("::label::\ngoto label\nbreak")
        .expect("the code must parse");
    let model = SemanticAnalyzer::new().analyze(&block).expect("analysis");
    let error = build_chunk(&block, &model).expect_err("break outside a loop must be rejected");
    assert!(matches!(
        error.kind,
        r4::cfg::CfgErrorKind::BreakOutsideLoop
    ));
}

#[test]
fn liveness_reads_every_prefix_and_type_shape() {
    let (cfg, model) = build_with_tokens(
        r#"
        local a = 1
        if condition then
            f()()
            (f)()
            f<<T>>()
            a.b()
            a[1]()
            local b = (a :: {number})
            local d = (a :: number?)
            local e = (a :: A & B)
            return a, b, d, e
        end
        return a
        "#,
    );

    let liveness = Liveness::analyze(&cfg, &model).expect("liveness must succeed");
    let a = symbol_named(&model, "a");
    let (then_block, _) = branch_blocks(&cfg, cfg.entry);
    assert!(liveness.is_live_in(then_block, a));
}
