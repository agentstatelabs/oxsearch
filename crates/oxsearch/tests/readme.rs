//! The README's quick start is `examples/quickstart.rs`, which CI compiles
//! and runs: this keeps them the same.

#[test]
fn the_quick_start_is_the_example() {
    let readme = include_str!("../../../README.md");
    let example = include_str!("../examples/quickstart.rs");
    let start = readme
        .find("```rust\n")
        .expect("the README has a Rust block")
        + "```rust\n".len();
    let end = start + readme[start..].find("```").expect("the block ends");
    let body: String = example
        .lines()
        .skip_while(|line| line.starts_with("//!"))
        .skip_while(|line| line.trim().is_empty())
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        &readme[start..end],
        body,
        "README.md's quick start differs from examples/quickstart.rs"
    );
}
