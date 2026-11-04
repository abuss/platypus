pub mod transport;

/// Resolve LSP server command for a given language, applying defaults.
/// Returns Option<Vec<String>> where:
/// - Some(vec![...]) = override command configured
/// - None if explicitly disabled (false or "") OR language has no default
/// - Default command if language not configured but has a default
/// Note: does NOT check if the binary exists on PATH — spawn failure is handled by Task 3.
pub fn command_for(cfg: &crate::config::Config, lang: &str) -> Option<Vec<String>> {
    // If explicitly configured (override or disable), use that entry
    if let Some(entry) = cfg.lsp.get(lang) {
        return entry.command.clone();
    }
    
    // Otherwise, check defaults table for absent keys
    let default_cmd = match lang {
        "rust" => Some(vec!["rust-analyzer"]),
        "c" => Some(vec!["clangd"]),
        "toml" => Some(vec!["taplo", "lsp", "stdio"]),
        "markdown" => Some(vec!["marksman"]),
        "bash" => Some(vec!["bash-language-server", "start"]),
        "json" => Some(vec!["vscode-json-language-server", "--stdio"]),
        _ => None,
    };
    
    default_cmd.map(|strs| strs.into_iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_table_all_six_languages() {
        let cfg = crate::config::Config::defaults();
        assert_eq!(command_for(&cfg, "rust"), Some(vec!["rust-analyzer".to_string()]));
        assert_eq!(command_for(&cfg, "c"), Some(vec!["clangd".to_string()]));
        assert_eq!(command_for(&cfg, "toml"), Some(vec!["taplo".to_string(), "lsp".to_string(), "stdio".to_string()]));
        assert_eq!(command_for(&cfg, "markdown"), Some(vec!["marksman".to_string()]));
        assert_eq!(command_for(&cfg, "bash"), Some(vec!["bash-language-server".to_string(), "start".to_string()]));
        assert_eq!(command_for(&cfg, "json"), Some(vec!["vscode-json-language-server".to_string(), "--stdio".to_string()]));
    }

    #[test]
    fn unknown_language_no_default() {
        let cfg = crate::config::Config::defaults();
        assert_eq!(command_for(&cfg, "python"), None);
    }

    #[test]
    fn override_beats_default() {
        let cfg = crate::config::from_str("[lsp]\nrust = \"my-ra --flag\"\n").unwrap();
        assert_eq!(command_for(&cfg, "rust"), Some(vec!["my-ra".to_string(), "--flag".to_string()]));
    }

    #[test]
    fn explicit_disabled_entry_beats_defaults() {
        let cfg = crate::config::from_str("[lsp]\ntoml = false\n").unwrap();
        // Key present with command=None → None (no default fallback)
        assert_eq!(command_for(&cfg, "toml"), None);
        
        // Different config, key absent → uses default
        let cfg2 = crate::config::Config::defaults();
        assert_eq!(command_for(&cfg2, "toml"), Some(vec!["taplo".to_string(), "lsp".to_string(), "stdio".to_string()]));
    }

    #[test]
    fn empty_string_disabled_beats_defaults() {
        let cfg = crate::config::from_str("[lsp]\ntoml = \"\"\n").unwrap();
        assert_eq!(command_for(&cfg, "toml"), None);
    }
}

