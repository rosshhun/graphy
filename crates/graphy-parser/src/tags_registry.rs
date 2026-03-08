use std::borrow::Cow;

use graphy_core::Language;

pub struct TagsLanguageConfig {
    pub ts_language: tree_sitter::Language,
    pub tags_query: Cow<'static, str>,
    pub language: Language,
}

/// Get a TagsLanguageConfig for a language by loading its dynamic grammar.
///
/// Returns `None` if the grammar is not installed. Install with:
/// `graphy lang add <name>`
pub fn tags_config_for_language(lang: Language) -> Option<TagsLanguageConfig> {
    crate::dynamic_loader::load_dynamic_grammar(lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_config_for_builtin_language_returns_none() {
        // Built-in languages (Python, TS, Rust, Svelte) use custom frontends,
        // not the tags system — so this should return None for them.
        let result = tags_config_for_language(Language::Python);
        assert!(result.is_none());
    }

    #[test]
    fn tags_config_for_uninstalled_dynamic_language() {
        // Languages that aren't installed should return None
        // (unless the test machine has Go grammar installed, which is unlikely in CI)
        // We just verify it doesn't panic
        let _result = tags_config_for_language(Language::Go);
    }

    #[test]
    fn tags_language_config_fields() {
        // Verify the struct fields are accessible
        // (compile-time check, no runtime assertion needed)
        fn _check_fields(config: TagsLanguageConfig) {
            let _lang = config.language;
            let _query = config.tags_query;
            let _ts = config.ts_language;
        }
    }
}
