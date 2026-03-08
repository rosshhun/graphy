/// Called cross-file from main.rs — should be alive.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Dead: pub but no callers anywhere.
pub fn dead_public_fn() -> String {
    String::from("unused")
}
