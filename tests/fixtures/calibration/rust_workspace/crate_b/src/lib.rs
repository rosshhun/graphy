use crate_a::shared_helper;

/// Uses shared_helper from crate_a — should trigger cross-crate resolution.
pub fn use_shared() -> i32 {
    shared_helper()
}
