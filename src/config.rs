use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// LSP server configuration
pub type LspCfg = BTreeMap<String, LspServerCfg>;

#[derive(Clone, Debug)]
pub struct LspServerCfg {
    pub command: Option<Vec<String>>,
}

/// Raw LSP value from TOML: string (command) or false (disabled)
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
enum LspValue {
    Command(String),
    Disabled(bool),
}

/// Configuration for editor behavior
#[derive(Clone, Debug)]
pub struct EditorCfg {
    pub line_numbers: bool,
    pub indent_width: u8,
    pub key_debug: bool,
    pub syntax: bool,
}

impl Default for EditorCfg {
    fn default() -> Self {
        EditorCfg {
            line_numbers: true,
            indent_width: 2,
            key_debug: false,
            syntax: true,
        }
    }
}

/// Configuration for sidebar
#[derive(Clone, Debug)]
pub struct SidebarCfg {
    pub width: u16,
    pub ignore: Vec<String>,
}

impl Default for SidebarCfg {
    fn default() -> Self {
        SidebarCfg {
            width: 32,
            ignore: Vec::new(),
        }
    }
}

/// Parsed configuration combining editor, sidebar, key mappings, and LSP servers
#[derive(Clone, Debug)]
pub struct Config {
    pub editor: EditorCfg,
    pub sidebar: SidebarCfg,
    pub keys: crate::keymap::KeyMap,
    pub lsp: LspCfg,
}

impl Config {
    pub fn defaults() -> Config {
        Config {
            editor: EditorCfg::default(),
            sidebar: SidebarCfg::default(),
            keys: HashMap::new(),
            lsp: BTreeMap::new(),
        }
    }
    
    pub fn command_for(&self, lang: &str) -> Option<Option<Vec<String>>> {
        self.lsp.get(lang).map(|cfg| cfg.command.clone())
    }
}

/// Internal raw structures for deserialization with serde
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawEditor {
    #[serde(default)]
    line_numbers: Option<bool>,
    #[serde(default)]
    indent_width: Option<u8>,
    #[serde(default)]
    key_debug: Option<bool>,
    #[serde(default)]
    syntax: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawSidebar {
    #[serde(default)]
    width: Option<u16>,
    #[serde(default)]
    ignore: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    editor: Option<RawEditor>,
    #[serde(default)]
    sidebar: Option<RawSidebar>,
    #[serde(default)]
    keys: Option<BTreeMap<String, String>>,
    #[serde(default)]
    lsp: Option<BTreeMap<String, LspValue>>,
}

/// Error type for configuration parsing and loading
#[derive(Clone, Debug)]
pub struct ConfigError {
    file: Option<PathBuf>,
    line: Option<usize>,
    message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "plat: config error")?;
        if let Some(file) = &self.file {
            write!(f, " in {}", file.display())?;
        }
        if let Some(line) = self.line {
            write!(f, ": line {}", line)?;
        }
        write!(f, ": {}", self.message)?;
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// Calculate edit distance (Levenshtein) between two strings, limited to 2
fn edit_distance(s: &str, t: &str) -> usize {
    if s == t {
        return 0;
    }
    if s.len() > t.len() + 2 || t.len() > s.len() + 2 {
        return 3; // too different
    }

    let mut prev = (0..=t.len()).collect::<Vec<_>>();
    let mut curr = vec![0; t.len() + 1];

    for (i, sc) in s.chars().enumerate() {
        curr[0] = i + 1;
        for (j, tc) in t.chars().enumerate() {
            let cost = if sc == tc { 0 } else { 1 };
            curr[j + 1] = std::cmp::min(
                std::cmp::min(curr[j] + 1, prev[j + 1] + 1),
                prev[j] + cost,
            );
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[t.len()]
}

/// Find similar field name with edit distance <= 2
fn suggest_field(unknown: &str, known_fields: &[&str]) -> Option<String> {
    known_fields
        .iter()
        .filter(|f| edit_distance(unknown, f) <= 2)
        .next()
        .map(|s| s.to_string())
}

/// Load configuration from a TOML string
pub fn from_str(text: &str) -> Result<Config, ConfigError> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| {
        let message = e.to_string();
        
        // Try to extract line number from error message (format: "line X at...")
        let line = message
            .split_whitespace()
            .skip_while(|w| w != &"line")
            .nth(1)
            .and_then(|s| s.parse::<usize>().ok());

        // Try to extract unknown field and provide suggestions
        if message.contains("unknown field") {
            if let Some(start) = message.find('`') {
                if let Some(end) = message[start + 1..].find('`') {
                    let field_name = &message[start + 1..start + 1 + end];
                    let suggestion = suggest_field_in_section(&message, field_name);
                    let enhanced = if let Some(sugg) = suggestion {
                        format!("{}, did you mean '{}'?", message, sugg)
                    } else {
                        message
                    };
                    return ConfigError {
                        file: None,
                        line,
                        message: enhanced,
                    };
                }
            }
        }

        ConfigError {
            file: None,
            line,
            message,
        }
    })?;

    let mut cfg = Config::defaults();

    // Apply editor settings with clamping
    if let Some(editor) = raw.editor {
        if let Some(ln) = editor.line_numbers {
            cfg.editor.line_numbers = ln;
        }
        if let Some(iw) = editor.indent_width {
            cfg.editor.indent_width = std::cmp::min(iw, 8);
        }
        if let Some(kd) = editor.key_debug {
            cfg.editor.key_debug = kd;
        }
        if let Some(syntax) = editor.syntax {
            cfg.editor.syntax = syntax;
        }
    }

    // Apply sidebar settings with clamping
    if let Some(sidebar) = raw.sidebar {
        if let Some(w) = sidebar.width {
            cfg.sidebar.width = w.max(crate::sidebar::MIN_WIDTH).min(crate::sidebar::MAX_WIDTH);
        }
        if let Some(ignore) = sidebar.ignore {
            cfg.sidebar.ignore = ignore;
        }
    }

    // Apply keys as-is
    if let Some(keys) = raw.keys {
        let mut parsed_keys = crate::keymap::KeyMap::new();
        for (spec_str, cmd_str) in keys {
            // Parse the key spec
            let key_spec = crate::keymap::parse_key_spec(&spec_str).map_err(|msg| ConfigError {
                file: None,
                line: None,
                message: msg,
            })?;
            
            // Parse the command (or None if "none")
            let command = if cmd_str == "none" {
                None
            } else {
                match crate::keymap::command_by_name(&cmd_str) {
                    Some(cmd) => Some(cmd),
                    None => {
                        return Err(ConfigError {
                            file: None,
                            line: None,
                            message: format!("unknown command '{}'", cmd_str),
                        });
                    }
                }
            };
            
            // Check for duplicates after normalization
            if parsed_keys.contains_key(&key_spec) {
                return Err(ConfigError {
                    file: None,
                    line: None,
                    message: format!("duplicate key binding for {} after normalization", spec_str),
                });
            }
            
            parsed_keys.insert(key_spec, command);
        }
        cfg.keys = parsed_keys;
    }

    // Apply LSP settings
    if let Some(lsp_map) = raw.lsp {
        for (lang, value) in lsp_map {
            let command = match value {
                LspValue::Command(s) => {
                    if s.is_empty() {
                        // Empty string → explicitly disabled
                        None
                    } else {
                        Some(s.split_whitespace().map(|s| s.to_string()).collect())
                    }
                }
                LspValue::Disabled(false) => {
                    // false → explicitly disabled
                    None
                }
                LspValue::Disabled(true) => {
                    // Serde won't produce this for untagged (true is not a string)
                    continue;
                }
            };
            cfg.lsp.insert(lang, LspServerCfg { command });
        }
    }

    Ok(cfg)
}

/// Determine which section an unknown field belongs to based on parse error message
fn suggest_field_in_section(_error_msg: &str, field_name: &str) -> Option<String> {
    // Try to infer which section from the error context
    let all_editor_fields = ["line_numbers", "indent_width", "key_debug", "syntax"];
    let all_sidebar_fields = ["width", "ignore"];

    // Check if field is close to any editor field
    if let Some(suggestion) = suggest_field(field_name, &all_editor_fields) {
        return Some(suggestion);
    }

    // Check if field is close to any sidebar field
    if let Some(suggestion) = suggest_field(field_name, &all_sidebar_fields) {
        return Some(suggestion);
    }

    None
}

/// Load configuration from a file path
pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let cfg = from_str(&text).map_err(|mut e| {
                e.file = Some(path.to_path_buf());
                e
            })?;
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::defaults()),
        Err(e) => Err(ConfigError {
            file: Some(path.to_path_buf()),
            line: None,
            message: e.to_string(),
        }),
    }
}

/// Get the configuration file path
pub fn config_path() -> PathBuf {
    // Try PLAT_CONFIG env first
    if let Ok(path) = std::env::var("PLAT_CONFIG") {
        return PathBuf::from(path);
    }

    // Try XDG_CONFIG_HOME
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("platypus/config.toml");
    }

    // Fall back to ~/.config/platypus/config.toml using HOME
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config/platypus/config.toml");
    }

    // Last resort (should rarely happen)
    PathBuf::from(".config/platypus/config.toml")
}

/// Load configuration from the default path
pub fn load() -> Result<Config, ConfigError> {
    let path = config_path();
    // If PLAT_CONFIG is explicitly set, a missing file is an error (not silent defaults).
    if std::env::var_os("PLAT_CONFIG").is_some() {
        match fs::read_to_string(&path) {
            Ok(text) => from_str(&text).map_err(|mut e| { e.file = Some(path.clone()); e }),
            Err(e) => Err(ConfigError { file: Some(path), line: None, message: e.to_string() }),
        }
    } else {
        load_from(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plat-config-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn example_config_parses_end_to_end() {
        let cfg = from_str(
            r#"
[editor]
line_numbers = false
indent_width = 4

[sidebar]
width = 40
ignore = ["dist", "*.min.js"]

[keys]
"C-Space" = "select_line"
"C-q" = "none"
"#,
        )
        .unwrap();
        assert!(!cfg.editor.line_numbers);
        assert_eq!(cfg.editor.indent_width, 4);
        assert_eq!(cfg.sidebar.width, 40);
        assert_eq!(cfg.sidebar.ignore, vec!["dist", "*.min.js"]);
        use crate::keymap::{Command, parse_key_spec};
        assert_eq!(cfg.keys.get(&parse_key_spec("C-Space").unwrap()), Some(&Some(Command::SelectLine)));
        assert_eq!(cfg.keys.get(&parse_key_spec("C-q").unwrap()), Some(&None));
    }

    #[test]
    fn defaults_apply_for_missing_sections_and_file() {
        let cfg = from_str("").unwrap();
        assert!(cfg.editor.line_numbers);
        assert_eq!(cfg.editor.indent_width, 2);
        assert_eq!(cfg.sidebar.width, 32);
        assert!(cfg.keys.is_empty());
        assert!(cfg.lsp.is_empty());
    }

    #[test]
    fn cosmetic_ranges_clamp_silently() {
        let cfg = from_str("[sidebar]\nwidth = 900\n[editor]\nindent_width = 99\n").unwrap();
        assert_eq!(cfg.sidebar.width, crate::sidebar::MAX_WIDTH);
        assert_eq!(cfg.editor.indent_width, 8);
    }

    #[test]
    fn sidebar_width_clamps_both_ends() {
        let cfg = from_str("[sidebar]\nwidth = 5\n").unwrap();
        assert_eq!(cfg.sidebar.width, crate::sidebar::MIN_WIDTH);
    }

    #[test]
    fn unknown_field_is_named_with_a_suggestion_and_line() {
        let err = from_str("[editor]\nline_number = false\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line_number"), "names the field: {msg}");
        assert!(msg.contains("line_numbers"), "suggests the right one: {msg}");
        assert!(msg.contains("line 2"), "reports the line: {msg}");
    }

    #[test]
    fn wrong_type_reports_the_line() {
        let err = from_str("[editor]\nline_numbers = \"yes\"\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn unreadable_file_is_an_error_not_defaults() {
        // Skip test if running as root (can read 0o000 files)
        if unsafe { libc::geteuid() } == 0 {
            return;
        }

        let path = tmp("unreadable.toml");
        fs::write(&path, "[editor]\nline_numbers = true\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        
        let result = load_from(&path);
        assert!(result.is_err(), "should error on unreadable file");
        
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_file(&path).ok();
    }

    #[test]
    fn plconfig_env_overrides_xdg_path() {
        let path = tmp("env_override.toml");
        fs::write(&path, "[editor]\nline_numbers = false\n").unwrap();

        unsafe {
            std::env::set_var("PLAT_CONFIG", &path);
        }

        let cfg = load().unwrap();
        assert!(!cfg.editor.line_numbers);

        unsafe {
            std::env::remove_var("PLAT_CONFIG");
        }

        fs::remove_file(&path).ok();
    }

    #[test]
    fn keys_table_builds_the_parsed_overlay() {
        let cfg = from_str(r#"[keys]
"C-Space" = "select_line"
"C-q" = "none"
"N" = "find_prev""#).unwrap();
        use crate::keymap::{Command, parse_key_spec};
        assert_eq!(cfg.keys.get(&parse_key_spec("C-Space").unwrap()), Some(&Some(Command::SelectLine)));
        assert_eq!(cfg.keys.get(&parse_key_spec("C-q").unwrap()), Some(&None));
        assert_eq!(cfg.keys.get(&parse_key_spec("S-n").unwrap()), Some(&Some(Command::FindPrev)));
    }

    #[test]
    fn the_same_binding_spelled_two_ways_is_a_duplicate_error() {
        let err = from_str("[keys]\n\"N\" = \"find_prev\"\n\"S-n\" = \"find_next\"").unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
        let err2 = from_str("[keys]\n\"C-S-q\" = \"quit\"\n\"C-q\" = \"save\"").unwrap_err();
        assert!(err2.to_string().contains("duplicate"), "{err2}");
    }

    #[test]
    fn unknown_command_and_bad_spec_are_named() {
        let e = from_str("[keys]\n\"C-q\" = \"selct_line\"").unwrap_err();
        assert!(e.to_string().contains("unknown command 'selct_line'"), "{e}");
        let e = from_str("[keys]\n\"Hyper-X\" = \"save\"").unwrap_err();
        assert!(e.to_string().contains("Hyper-X"), "{e}");
    }

    #[test]
    fn syntax_config_roundtrip_true_false_default() {
        let cfg = from_str("[editor]\nsyntax = false").unwrap();
        assert!(!cfg.editor.syntax);
        
        let cfg = from_str("[editor]\nsyntax = true").unwrap();
        assert!(cfg.editor.syntax);
        
        let cfg = from_str("").unwrap();
        assert!(cfg.editor.syntax, "default should be true");
    }

    #[test]
    fn syntax_field_in_did_you_mean() {
        let err = from_str("[editor]\nsyntaxx = false").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("syntaxx"), "names the unknown field: {msg}");
        assert!(msg.contains("syntax"), "suggests the right one: {msg}");
    }

    #[test]
    fn lsp_default_table_all_six_languages() {
        // Defaults are returned when lang key is absent from lsp map
        let cfg = Config::defaults();
        assert!(cfg.command_for("rust").is_none(), "no explicit entry");
        // When absent, defaults are returned by lsp::command_for, not Config::command_for
    }

    #[test]
    fn lsp_override_string_splits_whitespace() {
        let cfg = from_str("[lsp]\ntoml = \"taplo lsp stdio\"\n").unwrap();
        assert_eq!(cfg.command_for("toml"), Some(Some(vec!["taplo".to_string(), "lsp".to_string(), "stdio".to_string()])));
    }

    #[test]
    fn lsp_false_disables_language() {
        let cfg = from_str("[lsp]\ntoml = false\n").unwrap();
        assert_eq!(cfg.command_for("toml"), Some(None), "explicit disable returns Some(None)");
    }

    #[test]
    fn lsp_empty_string_disables_language() {
        let cfg = from_str("[lsp]\ntoml = \"\"\n").unwrap();
        assert_eq!(cfg.command_for("toml"), Some(None), "explicit disable returns Some(None)");
    }

    #[test]
    fn lsp_unknown_language_key_allowed() {
        let cfg = from_str("[lsp]\npython = \"pyright\"\n").unwrap();
        assert_eq!(cfg.command_for("python"), Some(Some(vec!["pyright".to_string()])));
    }

    #[test]
    fn lsp_unknown_section_still_errors() {
        let err = from_str("[lspp]\nrust = \"rust-analyzer\"\n").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn lsp_file_roundtrip() {
        let path = tmp("lsp_roundtrip.toml");
        fs::write(&path, "[lsp]\nrust = \"my-ra --flag\"\n").unwrap();
        
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.command_for("rust"), Some(Some(vec!["my-ra".to_string(), "--flag".to_string()])));
        
        fs::remove_file(&path).ok();
    }

    #[test]
    fn lsp_false_does_not_deserialize_as_command_false_string() {
        // This test verifies serde untagged string-before-bool ordering.
        // If bool came first, false would be parsed as bool and cause issues.
        let cfg = from_str("[lsp]\nrust = false\n").unwrap();
        assert_eq!(cfg.command_for("rust"), Some(None), "false should disable, not parse as string");
    }

    #[test]
    fn lsp_explicit_disabled_entry_beats_defaults() {
        let cfg = from_str("[lsp]\ntoml = false\n").unwrap();
        // Key is present in map with command=None → returns Some(None), NOT defaults
        assert_eq!(cfg.command_for("toml"), Some(None));
        
        // Contrast: absent key returns None from Config::command_for, defaults handled by lsp::command_for
        assert_eq!(cfg.command_for("rust"), None);
    }
}
