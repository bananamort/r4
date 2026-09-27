//! Micro-benchmarks for the r4 pipeline: parse, analyze, build a control-flow
//! graph, desugar to Lua 5.1, and generate code.
//!
//! Run them with `cargo bench`. Under `cargo test --all-targets` Criterion runs
//! each benchmark once in test mode, so a smoke run is part of CI.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use r4::generator::{LuaGenerator, ReadableLuaGenerator};
use r4::semantic::SemanticAnalyzer;
use r4::{Dialect, Parser};

/// A representative Luau program that exercises the parser, the analyzer, the
/// CFG builder and every desugaring pass.
const SOURCE: &str = r#"
local function clamp(value: number, minimum: number, maximum: number): number
    if value < minimum then
        return minimum
    elseif value > maximum then
        return maximum
    end
    return value
end
local total: number = 0
for i = 1, 10 do
    if i % 2 == 0 then
        continue
    end
    total += i
end
local label: string = if total > 20 then "big" else "small"
return clamp(total // 2, 0, 100), `label {label}`
"#;

fn parse(source: &str) -> r4::nodes::Block {
    Parser::default()
        .luau()
        .parse(source)
        .expect("the benchmark source is valid Luau")
}

fn analyze(block: &r4::nodes::Block) -> r4::semantic::SemanticModel {
    SemanticAnalyzer::new()
        .with_dialect(Dialect::Luau)
        .analyze(block)
        .expect("the benchmark source analyzes")
}

fn bench_parse_and_analyze(c: &mut Criterion) {
    c.bench_function("parse_and_analyze", |b| {
        b.iter(|| {
            let block = parse(black_box(SOURCE));
            black_box(analyze(&block));
        });
    });
}

fn bench_build_cfg(c: &mut Criterion) {
    let block = parse(SOURCE);
    let model = analyze(&block);
    c.bench_function("build_chunk", |b| {
        b.iter(|| {
            black_box(r4::cfg::build_chunk(black_box(&block), &model).expect("the cfg builds"));
        });
    });
}

fn bench_desugar(c: &mut Criterion) {
    let block = parse(SOURCE);
    c.bench_function("desugar_to_lua51", |b| {
        b.iter_batched(
            || block.clone(),
            |mut block| {
                r4::rules::desugar_to_lua51(&mut block).expect("the benchmark source lowers");
                black_box(block);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_generate(c: &mut Criterion) {
    let block = parse(SOURCE);
    c.bench_function("generate_readable", |b| {
        b.iter(|| {
            let mut generator = ReadableLuaGenerator::default();
            generator.write_block(black_box(&block));
            black_box(generator.into_string());
        });
    });
}

criterion_group!(
    benches,
    bench_parse_and_analyze,
    bench_build_cfg,
    bench_desugar,
    bench_generate
);
criterion_main!(benches);
