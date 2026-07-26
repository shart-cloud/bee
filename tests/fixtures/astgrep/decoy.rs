//! Fixture for 016-native-tools US1 (SC-007). Three occurrences of the SAME TEXT `v.unwrap()`:
//! one in live code, one in a comment, one inside a string literal. A regex finds all three; a
//! structural matcher must find only the first, because only that one is a method call in the
//! parse tree. That difference is the entire justification for the `ast_grep` tool.

fn real() -> u32 {
    let v: Option<u32> = None;
    v.unwrap()
}

// A comment mentioning v.unwrap() — text, not code.

fn quoted() -> &'static str {
    "v.unwrap() inside a string literal"
}
