<h1>Contributing</h1>

Thank you for your interest in contributing to this tool! There are different ways you can help this project become better.

- [Code of Conduct](#code-of-conduct)
- [Issues](#issues)
  - [Bugs](#bugs)
  - [Feature Requests](#feature-requests)
- [Submitting Code](#submitting-code)
- [Development](#development)
  - [Too Long Didn't Read](#too-long-didnt-read)
  - [Initial Setup](#initial-setup)
    - [Code Formatter](#code-formatter)
    - [Additional Linter](#additional-linter)
    - [Snapshot Testing Utility](#snapshot-testing-utility)
  - [Running Tests](#running-tests)
    - [Unit, semantic and control flow tests](#unit-semantic-and-control-flow-tests)
    - [AST fuzzer](#ast-fuzzer)
    - [Regression tests](#regression-tests)
    - [Snapshot tests](#snapshot-tests)
  - [Code formatter](#code-formatter-1)
  - [Clippy](#clippy)
  - [Architecture](#architecture)
  - [Help](#help)
    - [clippy is failing on CI but not locally](#clippy-is-failing-on-ci-but-not-locally)

# Code of Conduct

All contributors are expected to be respectful and constructive. Harassment or abusive behavior is not tolerated. If you experience or witness unacceptable behavior, please open an issue or contact a maintainer.

# Issues

Issues are used for various reasons. They're used to communicate and organize the project. Here's a few general ideas of what you can do:

* React to an issue: leaving a thumbs up or a heart emoji on an issue shows your interest for that issue. It can be **very** helpful when planning out the next features to add to the project.
* Replying to an issue: someone reported a bug and you found a way to reproduce it? Amazing! That's a really good thing to share to make it easier for maintainers to fix the problem. Giving feedback on a feature request or giving use case examples are also excellent examples of valuable help.
* Linking issues: you found duplicated issues? Write a comment that references the other issue, by using `#` followed by the issue number. For example, if an issue is duplicating the issue `7`, you can write a comment `Duplicate of #7`.

## Bugs

Before creating an issue to report a bug, search both opened and closed [issues](https://github.com/bananamort/r4/issues) to see if it does not exist already.

If you can, try to provide reproducing steps and the verbose output from the tool.

## Feature Requests

Have an idea about a new feature that could be added to the project? Create an issue and pitch it!

# Submitting Code

By contributing code to this project, you agree to license your contribution under the MIT license.

It's best to open an issue before contributing code, just to make sure that the new feature is ready to be integrated, or that the solution for a fix is appropriate for example. For trivial changes, like fixing a typo or re-wording the README, feel free to skip the issue and just submit a pull request.

# Development

If you are interested to work on r4, this section will help you get started. To ensure quality, r4 has automated checks running on each pull request. You'll learn how to verify these checks locally.

r4 is a semantically-aware and control-flow tracking Lua (5.1, 5.2) and Luau analysis and transformation engine.

## Too Long Didn't Read

In short, run these three commands often because they run really fast and give you the most actionable feedback.

```sh
cargo fmt
cargo test -q
cargo clippy --all-targets -- -D warnings
```

## Initial Setup

r4 is written in [Rust](https://www.rust-lang.org/), so you'll need to install the usual tools for Rust development. You can find how to install Rust [here](https://www.rust-lang.org/tools/install).

### Code Formatter

Code style is enforced on each pull request, so you will need to install `rustfmt` to auto-format your code.

```sh
rustup component add rustfmt
# adds the command `cargo fmt` to auto format
```

### Additional Linter

[Clippy](https://github.com/rust-lang/rust-clippy) is a static analysis tool that helps avoiding common mistakes in Rust code.

```sh
rustup component add clippy
# adds the command `cargo clippy` to verify the code
```

### Snapshot Testing Utility

r4 has snapshot tests using the [insta library](https://insta.rs/). To review or add new snapshot tests, you will need to install `cargo-insta`.

```sh
cargo install cargo-insta
# adds the command `cargo insta` to manage snapshots
```

## Running Tests

To run all the unit tests, semantic model tests, control flow analysis tests and regression tests, simply run the usual command with `cargo`:

```sh
cargo test
# or if the output is too verbose
cargo test -q
```

### Unit, semantic and control flow tests

The library unit tests cover the AST nodes, parser, generators, evaluator, semantic model and control flow graph:

```sh
cargo test --lib
cargo test --test semantic_test
cargo test --test cfg_test
```

### AST fuzzer

The property-based AST fuzzer generates random Lua/Luau ASTs, renders them with each generator and checks that the produced code parses back to the same AST. It runs in release-like iterations for a bounded amount of time:

```sh
cargo test --test fuzz_generator
```

By default each fuzz test runs for 1500 milliseconds. Set `FUZZ_DURATION_MILLISECONDS` to change the duration (a higher value explores more programs):

```sh
FUZZ_DURATION_MILLISECONDS=10000 cargo test --test fuzz_generator
```

### Regression tests

The parser regression tests parse Lua code that previously caused panics or stack overflows. The inputs live in `tests/fuzzed_test_cases/` and are collected in `tests/fuzzed.rs`:

```sh
cargo test --test fuzzed
```

When you fix a parser or AST conversion bug, add the smallest reproduction to `tests/fuzzed_test_cases/` and register it in the `generate_tests!` list.

### Snapshot tests

If there are snapshot test failures (because snapshots need to be updated, or new snapshots are added), use the following command to review them:

```sh
cargo insta review
```

The terminal will enter in interactive mode that lets you accept, reject or skip snapshots. More info can be found on the [insta quickstart](https://insta.rs/docs/quickstart/) documentation.

## Code formatter

To automatically format all the code, run:

```sh
cargo fmt
```

## Clippy

To get a full report and what needs to be fixed, run:

```sh
cargo clippy --all-targets -- -D warnings
```

The CI also runs clippy with all features enabled:

```sh
cargo clippy --all-targets --all-features -- -D warnings
```

## Architecture

* `src/nodes/`: Complete Lua 5.1/5.2 and Luau AST nodes.
* `src/parser.rs` & `src/ast_converter.rs`: Parsing via `full_moon`.
* `src/generator/`: Formatted code generators (`Readable`, `Dense`, `TokenBased`).
* `src/process/evaluator/`: Constant folding and purity analysis.
* `src/semantic/`: Symbol resolution, lexical scoping, def-use chains, upvalue capture and goto/label resolution.
* `src/cfg/`: Control flow graph builder, dominators, liveness, and reachability.
* `tests/ast_fuzzer/`: Property-based AST fuzzer used by `tests/fuzz_generator.rs`.
* `tests/fuzzed_test_cases/`: Regression inputs for the parser.

## Help

### clippy is failing on CI but not locally

You are probably running an older version of `rustc` or `clippy`. You can upgrade the Rust toolchain with `rustup`

```
rustup update
```

And re-run the clippy installation command to update clippy:

```
rustup component add clippy
```
