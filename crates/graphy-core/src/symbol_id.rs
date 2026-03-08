use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::gir::NodeKind;

/// Content-addressable symbol identifier.
///
/// Computed from (file_path, name, kind, start_line) so the same symbol
/// parsed twice produces the same ID.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SymbolId(u64);

impl SymbolId {
    pub fn new(file_path: &Path, name: &str, kind: NodeKind, start_line: u32) -> Self {
        let mut hasher = StableHasher::new();
        file_path.to_string_lossy().as_ref().hash(&mut hasher);
        name.hash(&mut hasher);
        (kind as u32).hash(&mut hasher);
        start_line.hash(&mut hasher);
        Self(hasher.finish())
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SymbolId({:016x})", self.0)
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// A simple stable hasher using FNV-1a so IDs are deterministic across runs.
struct StableHasher {
    state: u64,
}

impl StableHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf29ce484222325, // FNV offset basis
        }
    }
}

impl Hasher for StableHasher {
    fn finish(&self) -> u64 {
        self.state
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= byte as u64;
            self.state = self.state.wrapping_mul(0x100000001b3); // FNV prime
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_ids() {
        let id1 = SymbolId::new(Path::new("foo.py"), "bar", NodeKind::Function, 10);
        let id2 = SymbolId::new(Path::new("foo.py"), "bar", NodeKind::Function, 10);
        assert_eq!(id1, id2);
    }

    #[test]
    fn different_inputs_different_ids() {
        let id1 = SymbolId::new(Path::new("foo.py"), "bar", NodeKind::Function, 10);
        let id2 = SymbolId::new(Path::new("foo.py"), "baz", NodeKind::Function, 10);
        assert_ne!(id1, id2);
    }

    #[test]
    fn different_file_different_id() {
        let id1 = SymbolId::new(Path::new("a.py"), "func", NodeKind::Function, 1);
        let id2 = SymbolId::new(Path::new("b.py"), "func", NodeKind::Function, 1);
        assert_ne!(id1, id2);
    }

    #[test]
    fn different_kind_different_id() {
        let id1 = SymbolId::new(Path::new("a.py"), "Foo", NodeKind::Function, 1);
        let id2 = SymbolId::new(Path::new("a.py"), "Foo", NodeKind::Class, 1);
        assert_ne!(id1, id2);
    }

    #[test]
    fn different_line_different_id() {
        let id1 = SymbolId::new(Path::new("a.py"), "func", NodeKind::Function, 1);
        let id2 = SymbolId::new(Path::new("a.py"), "func", NodeKind::Function, 2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn unicode_inputs_deterministic() {
        let id1 = SymbolId::new(Path::new("日本語/ファイル.py"), "関数", NodeKind::Function, 1);
        let id2 = SymbolId::new(Path::new("日本語/ファイル.py"), "関数", NodeKind::Function, 1);
        assert_eq!(id1, id2);
    }

    #[test]
    fn empty_name_deterministic() {
        let id1 = SymbolId::new(Path::new("a.py"), "", NodeKind::Function, 0);
        let id2 = SymbolId::new(Path::new("a.py"), "", NodeKind::Function, 0);
        assert_eq!(id1, id2);
    }

    #[test]
    fn display_is_hex() {
        let id = SymbolId::new(Path::new("a.py"), "f", NodeKind::Function, 1);
        let s = format!("{}", id);
        assert_eq!(s.len(), 16); // 16 hex chars for u64
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn debug_format() {
        let id = SymbolId::new(Path::new("a.py"), "f", NodeKind::Function, 1);
        let s = format!("{:?}", id);
        assert!(s.starts_with("SymbolId("));
        assert!(s.ends_with(')'));
    }

    #[test]
    fn as_u64_matches_internal() {
        let id = SymbolId::new(Path::new("a.py"), "f", NodeKind::Function, 1);
        let val = id.as_u64();
        assert_ne!(val, 0);
    }

    #[test]
    fn ordering_is_consistent() {
        let id1 = SymbolId::new(Path::new("a.py"), "a", NodeKind::Function, 1);
        let id2 = SymbolId::new(Path::new("a.py"), "b", NodeKind::Function, 1);
        // Just check that ordering is total and consistent
        let cmp1 = id1.cmp(&id2);
        let cmp2 = id2.cmp(&id1);
        assert_eq!(cmp1.reverse(), cmp2);
    }

    #[test]
    fn max_line_number() {
        // Should not panic with max u32
        let id = SymbolId::new(Path::new("a.py"), "f", NodeKind::Function, u32::MAX);
        assert_ne!(id.as_u64(), 0);
    }

    #[test]
    fn serde_round_trip() {
        let id = SymbolId::new(Path::new("a.py"), "func", NodeKind::Function, 42);
        let json = serde_json::to_string(&id).unwrap();
        let back: SymbolId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn collision_resistance_realistic() {
        // Generate IDs for realistic function names in the same file
        // and ensure no collisions
        let mut ids = std::collections::HashSet::new();
        let names = [
            "init", "new", "get", "set", "update", "delete", "create",
            "find", "search", "validate", "parse", "render", "handle",
            "process", "transform", "build", "run", "start", "stop",
            "close", "open", "read", "write", "flush", "reset",
        ];
        for (i, name) in names.iter().enumerate() {
            let id = SymbolId::new(
                Path::new("src/lib.rs"),
                name,
                NodeKind::Function,
                (i * 10) as u32,
            );
            assert!(ids.insert(id), "collision on {}", name);
        }
    }
}
