use r4::nodes::Block;
use r4::{Parser, ParserError};
use std::time::{Duration, Instant};

#[allow(dead_code)]
pub fn parse_input(input: &str) -> Block {
    match Parser::default().parse(input) {
        Ok(block) => block,
        Err(error) => panic!("could not parse content: {:?}\ncontent:\n{}", error, input),
    }
}

#[allow(dead_code)]
pub fn try_parse_input(input: &str) -> Result<Block, ParserError> {
    // The generators produce Luau syntax, so parse it as Luau.
    Parser::default().luau().parse(input)
}

#[allow(dead_code)]
pub fn run_for_minimum_time<F: Fn()>(duration: Duration, func: F) {
    let start = Instant::now();
    loop {
        func();
        if Instant::now().duration_since(start) > duration {
            break;
        }
    }
}
