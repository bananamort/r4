#![cfg(not(coverage))]

// this test file is for collecting fuzzed Lua code that was causing issues that have been fixed

macro_rules! generate_tests {
    ($( $name:ident => $file_name:literal ),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                use r4::Parser;

                let input = include_str!($file_name);

                match Parser::default().parse(&input) {
                    Ok(_block) => (),
                    Err(error) => panic!("could not parse content: {:?}\ncontent:\n{}", error, input),
                }
            }
        )*
    }
}

generate_tests!(
    ast_conversion_stackoverflow_in_debug => "./fuzzed_test_cases/a.lua",
    fuzzed_test_case_b => "./fuzzed_test_cases/b.lua",
    fuzzed_test_case_c => "./fuzzed_test_cases/c.lua",
    full_moon_parser_stackoverflow => "./fuzzed_test_cases/full_moon_stackoverflow.lua",
);
