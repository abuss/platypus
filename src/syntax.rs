use ratatui::style::{Color, Modifier, Style};
use std::path::Path;

/// Maps file extensions to language names (case-insensitive).
pub fn language_for(path: Option<&Path>) -> Option<&'static str> {
    let path = path?;
    let ext = path
        .extension()?
        .to_str()?
        .to_lowercase();

    Some(match ext.as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "md" => "markdown",
        "json" => "json",
        "c" | "h" => "c",
        "sh" | "bash" => "bash",
        _ => return None,
    })
}

/// Maps a scope to its style using longest-prefix matching.
/// Palette:
/// - keyword* → LightMagenta
/// - function + function.method → LightCyan
/// - type + type.builtin → Yellow
/// - string* + char → Green
/// - number + constant → LightBlue
/// - comment → DarkGrey + ITALIC
/// - property + field → LightYellow
/// - error → Red
/// - everything else → None
pub fn style_for_scope(scope: &str) -> Option<Style> {
    // Longest-prefix table: iterate in order, take the first (longest) match
    let table = [
        ("keyword", Style::default().fg(Color::LightMagenta)),
        ("function.method", Style::default().fg(Color::LightCyan)),
        ("function", Style::default().fg(Color::LightCyan)),
        ("type.builtin", Style::default().fg(Color::Yellow)),
        ("type", Style::default().fg(Color::Yellow)),
        ("string", Style::default().fg(Color::Green)),
        ("char", Style::default().fg(Color::Green)),
        ("number", Style::default().fg(Color::LightBlue)),
        ("constant", Style::default().fg(Color::LightBlue)),
        ("comment", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
        ("property", Style::default().fg(Color::LightYellow)),
        ("field", Style::default().fg(Color::LightYellow)),
        ("error", Style::default().fg(Color::Red)),
    ];

    for (prefix, style) in &table {
        if scope.starts_with(prefix) {
            return Some(*style);
        }
    }

    None
}

/// Wrapper to hold a tree-sitter language reference.
pub struct LanguageCtor(pub &'static tree_sitter::Language);

/// Returns a tree_sitter::Language for the given language name.
pub fn grammar(lang: &'static str) -> Option<tree_sitter::Language> {
    unsafe {
        Some(match lang {
            "rust" => {
                // SAFETY: LanguageFn wraps an extern "C" fn returning `*const ()` (opaque TSLanguage pointer).
                // Language is #[repr(transparent)] over `*const ffi::TSLanguage` (same opaque type).
                // Transmute safely bridges them. Tree-sitter 0.25+ uses From<LanguageFn> impl;
                // 0.20 requires transmute as the only bridge.
                std::mem::transmute::<*const (), tree_sitter::Language>(tree_sitter_rust::LANGUAGE.into_raw()())
            }
            "toml" => tree_sitter_toml::language(),
            "markdown" => std::mem::transmute::<*const (), tree_sitter::Language>(tree_sitter_md::LANGUAGE.into_raw()()),
            "json" => std::mem::transmute::<*const (), tree_sitter::Language>(tree_sitter_json::LANGUAGE.into_raw()()),
            "c" => std::mem::transmute::<*const (), tree_sitter::Language>(tree_sitter_c::LANGUAGE.into_raw()()),
            "bash" => std::mem::transmute::<*const (), tree_sitter::Language>(tree_sitter_bash::LANGUAGE.into_raw()()),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_map_is_case_insensitive() {
        assert_eq!(language_for(Some(Path::new("a.RS"))), Some("rust"));
        assert_eq!(language_for(Some(Path::new("x/README.MD"))), Some("markdown"));
        assert_eq!(language_for(Some(Path::new("k.h"))), Some("c"));
        assert_eq!(language_for(Some(Path::new("notes.txt"))), None);
        assert_eq!(language_for(None), None);
    }

    #[test]
    fn palette_scopes_resolve() {
        assert_eq!(
            style_for_scope("keyword.function"),
            Some(Style::default().fg(Color::LightMagenta))
        );
        assert_eq!(
            style_for_scope("comment"),
            Some(Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC))
        );
        assert_eq!(style_for_scope("variable"), None);
        assert_eq!(style_for_scope("nonexistent.scope"), None);
        assert_eq!(style_for_scope("punctuation.bracket"), None);
        assert_eq!(style_for_scope("punctuation.delimiter"), None);
    }

    #[test]
    fn all_six_grammars_construct() {
        for name in ["rust", "toml", "markdown", "json", "c", "bash"] {
            assert!(grammar(name).is_some(), "{name} missing");
        }
    }
}
