mod utils;

fn main() {
    let result = utils::add(1, 2);
    println!("Result: {}", result);
}

/// Called from main — should be alive.
pub fn entry_point_helper() -> i32 {
    42
}

/// Dead: no callers, private, no references.
fn truly_dead() -> bool {
    false
}

#[test]
fn test_entry() {
    assert_eq!(entry_point_helper(), 42);
}
