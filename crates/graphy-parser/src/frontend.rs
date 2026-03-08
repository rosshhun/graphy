use std::path::Path;

use anyhow::Result;
use graphy_core::ParseOutput;

/// Trait that all language frontends implement.
///
/// A frontend takes a file path + source code and produces GIR nodes and edges.
pub trait LanguageFrontend {
    fn parse(&self, path: &Path, source: &str) -> Result<ParseOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A trivial implementation to verify the trait is object-safe and usable.
    struct DummyFrontend;
    impl LanguageFrontend for DummyFrontend {
        fn parse(&self, _path: &Path, _source: &str) -> Result<ParseOutput> {
            Ok(ParseOutput::default())
        }
    }

    #[test]
    fn dummy_frontend_returns_empty_output() {
        let frontend = DummyFrontend;
        let output = frontend.parse(Path::new("test.py"), "").unwrap();
        assert!(output.nodes.is_empty());
        assert!(output.edges.is_empty());
    }

    #[test]
    fn trait_is_object_safe() {
        // Verify LanguageFrontend can be used as a trait object
        let frontend: Box<dyn LanguageFrontend> = Box::new(DummyFrontend);
        let output = frontend.parse(Path::new("test.py"), "x = 1").unwrap();
        assert!(output.nodes.is_empty());
    }

    #[test]
    fn frontend_handles_unicode_source() {
        let frontend = DummyFrontend;
        let output = frontend.parse(Path::new("日本語.py"), "変数 = '値'").unwrap();
        assert!(output.nodes.is_empty());
    }
}
