/// Called from crate_b — should be alive after cross-crate resolution.
pub fn shared_helper() -> i32 {
    42
}

/// Dead: pub but never called from anywhere.
pub fn dead_in_crate_a() -> bool {
    false
}
