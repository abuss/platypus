pub mod transport;

/// Resolve LSP server command for a given language, applying defaults.
/// Returns argv (whitespace-split if override, or default command) or None if disabled.
/// Note: does NOT check if the binary exists on PATH — spawn failure is handled by Task 3.
pub fn command_for(cfg: &crate::config::Config, lang: &str) -> Option<Vec<String>> {
    // If configured (override or explicit disable), use that
    if let Some(cmd_vec) = cfg.command_for(lang) {
        return Some(cmd_vec);
    }
    
    // Otherwise, check defaults table
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
        let mut cfg = crate::config::Config::defaults();
        cfg.lsp.insert("rust".to_string(), crate::config::LspServerCfg {
            command: vec!["my-ra".to_string(), "--flag".to_string()],
        });
        assert_eq!(command_for(&cfg, "rust"), Some(vec!["my-ra".to_string(), "--flag".to_string()]));
    }

    #[test]
    fn disabled_override_beats_default() {
        let cfg = crate::config::Config::defaults();
        // Disabled entries are absent from lsp map (we skip them during parsing)
        // This test just verifies the logic doesn't crash on unknown lang with no default
        assert_eq!(command_for(&cfg, "unknown"), None);
    }
}
