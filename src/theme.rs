use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Theme configuration section: UI and syntax color overrides
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct ThemeSection {
    #[serde(default)]
    pub ui: BTreeMap<String, StyleSpec>,
    #[serde(default)]
    pub syntax: BTreeMap<String, StyleSpec>,
}

/// Parse a color string into a ratatui Color
pub fn parse_color(s: &str) -> Result<Color, String> {
    let s = s.trim();
    
    // Named colors (case-insensitive)
    match s.to_lowercase().as_str() {
        "red" => return Ok(Color::Red),
        "green" => return Ok(Color::Green),
        "yellow" => return Ok(Color::Yellow),
        "blue" => return Ok(Color::Blue),
        "magenta" => return Ok(Color::Magenta),
        "cyan" => return Ok(Color::Cyan),
        "white" => return Ok(Color::White),
        "black" => return Ok(Color::Black),
        "bright-red" | "light-red" => return Ok(Color::LightRed),
        "bright-green" | "light-green" => return Ok(Color::LightGreen),
        "bright-yellow" | "light-yellow" => return Ok(Color::LightYellow),
        "bright-blue" | "light-blue" => return Ok(Color::LightBlue),
        "bright-magenta" | "light-magenta" => return Ok(Color::LightMagenta),
        "bright-cyan" | "light-cyan" => return Ok(Color::LightCyan),
        "bright-white" => return Ok(Color::White),
        "gray" | "grey" | "light-gray" | "light-grey" => return Ok(Color::Gray),
        "dark-gray" | "dark-grey" => return Ok(Color::DarkGray),
        "dark-cyan" => return Ok(Color::Cyan),
        "dark-yellow" => return Ok(Color::Yellow),
        "reset" => return Ok(Color::Reset),
        _ => {}
    }
    
    // Hex color: #RRGGBB
    if s.starts_with('#') && s.len() == 7 {
        if let Ok(rgb) = u32::from_str_radix(&s[1..], 16) {
            let r = ((rgb >> 16) & 0xff) as u8;
            let g = ((rgb >> 8) & 0xff) as u8;
            let b = (rgb & 0xff) as u8;
            return Ok(Color::Rgb(r, g, b));
        } else {
            return Err(format!("invalid hex color: {}", s));
        }
    }
    
    // Indexed color: 0..=255
    if let Ok(idx) = s.parse::<i64>() {
        if idx >= 0 && idx <= 255 {
            return Ok(Color::Indexed(idx as u8));
        } else {
            return Err(format!("color index out of range: {}", s));
        }
    }
    
    Err(format!("invalid color: {}", s))
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StyleSpec {
    Color(String),
    Index(i64),
    Full(StyleTable),
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct StyleTable {
    #[serde(alias = "color", default)]
    pub fg: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub bold: Option<bool>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub underline: Option<bool>,
}

impl StyleSpec {
    pub fn apply(&self, base: Style, warn: &mut Vec<String>) -> Style {
        match self {
            StyleSpec::Color(s) => {
                match parse_color(s) {
                    Ok(color) => base.fg(color),
                    Err(e) => {
                        warn.push(e);
                        base
                    }
                }
            }
            StyleSpec::Index(idx) => {
                if *idx >= 0 && *idx <= 255 {
                    base.fg(Color::Indexed(*idx as u8))
                } else {
                    warn.push(format!("color index out of range: {}", idx));
                    base
                }
            }
            StyleSpec::Full(table) => {
                let mut result = base;
                
                if let Some(fg_str) = &table.fg {
                    match parse_color(fg_str) {
                        Ok(color) => result = result.fg(color),
                        Err(e) => warn.push(e),
                    }
                }
                
                if let Some(bg_str) = &table.bg {
                    match parse_color(bg_str) {
                        Ok(color) => result = result.bg(color),
                        Err(e) => warn.push(e),
                    }
                }
                
                if let Some(bold) = table.bold {
                    if bold {
                        result = result.add_modifier(Modifier::BOLD);
                    } else {
                        result = result.remove_modifier(Modifier::BOLD);
                    }
                }
                
                if let Some(italic) = table.italic {
                    if italic {
                        result = result.add_modifier(Modifier::ITALIC);
                    } else {
                        result = result.remove_modifier(Modifier::ITALIC);
                    }
                }
                
                if let Some(underline) = table.underline {
                    if underline {
                        result = result.add_modifier(Modifier::UNDERLINED);
                    } else {
                        result = result.remove_modifier(Modifier::UNDERLINED);
                    }
                }
                
                result
            }
        }
    }
}

/// Syntax scope → Style lookup; entries sorted by key length DESC so longest prefix wins
#[derive(Clone, Debug)]
pub struct SyntaxPalette {
    entries: Vec<(String, Style)>,
}

impl SyntaxPalette {
    pub fn style_for_scope(&self, scope: &str) -> Option<Style> {
        for (prefix, style) in &self.entries {
            if scope.starts_with(prefix.as_str()) {
                return Some(*style);
            }
        }
        None
    }

    fn from_sorted(mut entries: Vec<(String, Style)>) -> Self {
        entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        SyntaxPalette { entries }
    }
}

/// Fully resolved theme palette
#[derive(Clone, Debug)]
pub struct Palette {
    pub status_bar: Style,
    pub tab_active: Style,
    pub tab_inactive: Style,
    pub selection: Style,
    pub completion_sel: Style,
    /// Background rectangle behind unselected completion/code-action rows
    /// (#40): makes the dropdown read as its own area over the editor.
    pub menu_bg: Style,
    pub sidebar_sel: Style,
    pub find_match: Style,
    pub diag_error: Style,
    pub diag_warn: Style,
    pub diag_info: Style,
    pub diag_hint: Style,
    pub palette_sel: Style,
    /// Base fg/bg for the editor text area (incl. gutter, cells past EOL);
    /// syntax/selection/diag styles patch on top of it.
    pub default: Style,
    /// Emphasis for the active line's gutter number (Helix-style).
    pub current_line_no: Style,
    /// Plain gutter line numbers (empty built-in: they inherit the `default` fg).
    pub line_no: Style,
    /// Dir names in the sidebar (built-in: bold).
    pub sidebar_dir: Style,
    /// Mode-word block ("EDIT ", "ACTION+") on the status line.
    pub status_mode: Style,
    pub syntax: SyntaxPalette,
}

impl Palette {
    pub fn dark() -> Palette {
        let syntax_entries = vec![
            ("keyword".to_string(), Style::default().fg(Color::LightMagenta)),
            ("function.method".to_string(), Style::default().fg(Color::LightCyan)),
            ("function".to_string(), Style::default().fg(Color::LightCyan)),
            ("type.builtin".to_string(), Style::default().fg(Color::Yellow)),
            ("type".to_string(), Style::default().fg(Color::Yellow)),
            ("string".to_string(), Style::default().fg(Color::Green)),
            ("char".to_string(), Style::default().fg(Color::Green)),
            ("number".to_string(), Style::default().fg(Color::LightBlue)),
            ("constant".to_string(), Style::default().fg(Color::LightBlue)),
            ("comment".to_string(), Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
            ("property".to_string(), Style::default().fg(Color::LightYellow)),
            ("field".to_string(), Style::default().fg(Color::LightYellow)),
            ("error".to_string(), Style::default().fg(Color::Red)),
            // scopes added for python/go/lua highlighting
            ("conditional".to_string(), Style::default().fg(Color::LightMagenta)),
            ("repeat".to_string(), Style::default().fg(Color::LightMagenta)),
            ("method".to_string(), Style::default().fg(Color::LightCyan)),
            ("boolean".to_string(), Style::default().fg(Color::LightBlue)),
        ];
        Palette {
            status_bar: Style::default(),
            tab_active: Style::default(),
            tab_inactive: Style::default(),
            selection: Style::default().bg(Color::Blue),
            completion_sel: Style::default().bg(Color::Blue),
            menu_bg: Style::default().bg(Color::DarkGray),
            sidebar_sel: Style::default().bg(Color::Blue),
            find_match: Style::default().add_modifier(Modifier::UNDERLINED),
            diag_error: Style::default().fg(Color::Red),
            diag_warn: Style::default().fg(Color::Yellow),
            diag_info: Style::default().fg(Color::DarkGray),
            diag_hint: Style::default().fg(Color::DarkGray),
            palette_sel: Style::default().bg(Color::Blue),
            default: Style::default(),
            current_line_no: Style::default().add_modifier(Modifier::BOLD).add_modifier(Modifier::UNDERLINED),
            line_no: Style::default(),
            sidebar_dir: Style::default().add_modifier(Modifier::BOLD),
            status_mode: Style::default().add_modifier(Modifier::REVERSED),
            syntax: SyntaxPalette::from_sorted(syntax_entries),
        }
    }

    pub fn light() -> Palette {
        // Start from dark, apply light overrides per spec §Built-in palettes
        let dark = Palette::dark();
        let mut w = Vec::new();
        let st = |fg: Option<&str>, bg: Option<&str>, bold: Option<bool>, italic: Option<bool>| StyleSpec::Full(StyleTable {
            fg: fg.map(|s| s.to_string()), bg: bg.map(|s| s.to_string()), bold, italic, underline: None,
        });
        let light_ui: &[(&str, StyleSpec)] = &[
            ("status_bar",    st(Some("black"), Some("gray"), None, None)),
            ("tab_active",    st(None, None, Some(true), None)),
            ("selection",     st(None, Some("light-gray"), None, None)),
            ("completion_sel",st(None, Some("light-gray"), None, None)),
            ("menu_bg",       st(None, Some("light-gray"), None, None)),
            ("sidebar_sel",   st(None, Some("light-gray"), None, None)),
            ("palette_sel",   st(None, Some("light-gray"), None, None)),
            ("diag_warn",     StyleSpec::Color("dark-yellow".to_string())),
            ("diag_info",     StyleSpec::Color("dark-gray".to_string())),
            ("diag_hint",     StyleSpec::Color("dark-gray".to_string())),
        ];
        let mut p = dark;
        for (role, spec) in light_ui {
            match *role {
                "status_bar"    => p.status_bar    = spec.apply(p.status_bar, &mut w),
                "tab_active"    => p.tab_active    = spec.apply(p.tab_active, &mut w),
                "selection"     => p.selection     = spec.apply(p.selection, &mut w),
                "completion_sel"=> p.completion_sel= spec.apply(p.completion_sel, &mut w),
                "menu_bg"      => p.menu_bg       = spec.apply(p.menu_bg, &mut w),
                "sidebar_sel"   => p.sidebar_sel   = spec.apply(p.sidebar_sel, &mut w),
                "palette_sel"   => p.palette_sel   = spec.apply(p.palette_sel, &mut w),
                "diag_warn"     => p.diag_warn     = spec.apply(p.diag_warn, &mut w),
                "diag_info"     => p.diag_info     = spec.apply(p.diag_info, &mut w),
                "diag_hint"     => p.diag_hint     = spec.apply(p.diag_hint, &mut w),
                _ => {}
            }
        }
        // Syntax overrides: merge onto dark 17-entry table
        let syntax_over: &[(&str, StyleSpec)] = &[
            ("keyword",          StyleSpec::Color("blue".to_string())),
            ("function",         StyleSpec::Color("blue".to_string())),
            ("function.method",  StyleSpec::Color("blue".to_string())),
            ("type",             StyleSpec::Color("dark-cyan".to_string())),
            ("type.builtin",     StyleSpec::Color("dark-cyan".to_string())),
            ("string",           StyleSpec::Color("green".to_string())),
            ("char",             StyleSpec::Color("green".to_string())),
            ("number",           StyleSpec::Color("magenta".to_string())),
            ("constant",         StyleSpec::Color("magenta".to_string())),
            ("comment",          StyleSpec::Full(StyleTable { fg: Some("dark-gray".to_string()), bg: None, bold: None, italic: Some(true), underline: None })),
            ("property",         StyleSpec::Color("dark-yellow".to_string())),
            ("field",            StyleSpec::Color("dark-yellow".to_string())),
            ("error",            StyleSpec::Color("red".to_string())),
            ("conditional",      StyleSpec::Color("blue".to_string())),
            ("repeat",           StyleSpec::Color("blue".to_string())),
            ("method",           StyleSpec::Color("blue".to_string())),
            ("boolean",          StyleSpec::Color("magenta".to_string())),
        ];
        let base_entries = p.syntax.entries;
        let over_map: std::collections::BTreeMap<String, StyleSpec> = syntax_over.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        p.syntax = merge_syntax(base_entries, &over_map, &mut w);
        p
    }
}

const UI_ROLES: &[&str] = &[
    "status_bar", "tab_active", "tab_inactive", "selection",
    "completion_sel", "menu_bg", "sidebar_sel", "find_match",
    "diag_error", "diag_warn", "diag_info", "diag_hint", "default",
    "current_line_no", "status_mode", "palette_sel", "line_no", "sidebar_dir",
];

fn merge_syntax(
    base: Vec<(String, Style)>,
    over: &BTreeMap<String, StyleSpec>,
    warn: &mut Vec<String>,
) -> SyntaxPalette {
    let base_map: BTreeMap<String, Style> = base.into_iter().collect();
    let mut result: Vec<(String, Style)> = Vec::new();
    // Keep all base entries, applying overrides where key matches
    for (k, v) in &base_map {
        let style = if let Some(spec) = over.get(k) {
            spec.apply(*v, warn)
        } else {
            *v
        };
        result.push((k.clone(), style));
    }
    // Add override entries not in base
    for (k, spec) in over {
        if !base_map.contains_key(k) {
            result.push((k.clone(), spec.apply(Style::default(), warn)));
        }
    }
    SyntaxPalette::from_sorted(result)
}

fn apply_ui_overrides(palette: &mut Palette, ui: &BTreeMap<String, StyleSpec>, notes: &mut Vec<String>) {
    let mut bad_colors = 0usize;
    for (key, spec) in ui {
        if !UI_ROLES.contains(&key.as_str()) {
            notes.push(format!("unknown theme ui key: {key}"));
            continue;
        }
        let mut w = Vec::new();
        match key.as_str() {
            "status_bar"    => palette.status_bar    = spec.apply(palette.status_bar, &mut w),
            "tab_active"    => palette.tab_active    = spec.apply(palette.tab_active, &mut w),
            "tab_inactive"  => palette.tab_inactive  = spec.apply(palette.tab_inactive, &mut w),
            "selection"     => palette.selection     = spec.apply(palette.selection, &mut w),
            "completion_sel"=> palette.completion_sel= spec.apply(palette.completion_sel, &mut w),
            "menu_bg"      => palette.menu_bg       = spec.apply(palette.menu_bg, &mut w),
            "sidebar_sel"   => palette.sidebar_sel   = spec.apply(palette.sidebar_sel, &mut w),
            "find_match"    => palette.find_match    = spec.apply(palette.find_match, &mut w),
            "diag_error"    => palette.diag_error    = spec.apply(palette.diag_error, &mut w),
            "diag_warn"     => palette.diag_warn     = spec.apply(palette.diag_warn, &mut w),
            "diag_info"     => palette.diag_info     = spec.apply(palette.diag_info, &mut w),
            "diag_hint"     => palette.diag_hint     = spec.apply(palette.diag_hint, &mut w),
            "default"       => palette.default       = spec.apply(palette.default, &mut w),
            "current_line_no" => palette.current_line_no = spec.apply(palette.current_line_no, &mut w),
            "line_no" => palette.line_no = spec.apply(palette.line_no, &mut w),
            "sidebar_dir" => palette.sidebar_dir = spec.apply(palette.sidebar_dir, &mut w),
            "status_mode"     => palette.status_mode     = spec.apply(palette.status_mode, &mut w),
            "palette_sel"   => palette.palette_sel   = spec.apply(palette.palette_sel, &mut w),
            _ => {}
        }
        bad_colors += w.len();
    }
    if bad_colors > 0 {
        notes.push(format!("{bad_colors} invalid color value(s) in theme config"));
    }
}

pub fn resolve_in(cfg: &crate::config::Config, themes_dir: &Path) -> (Palette, Vec<String>) {
    let name = &cfg.editor.theme_name;
    let mut notes = Vec::new();

    // Determine base palette from built-in name (before possible file shadow)
    let base = match name.as_str() {
        "light" => Palette::light(),
        _ => Palette::dark(),
    };

    // Try to load theme file (shadows built-in including dark/light)
    let theme_path = themes_dir.join(format!("{name}.toml"));
    let file_section: Option<ThemeSection> = match std::fs::read_to_string(&theme_path) {
        Ok(text) => match toml::from_str::<ThemeSection>(&text) {
            Ok(s) => Some(s),
            Err(e) => { notes.push(format!("theme file error: {e}")); None }
        },
        Err(_) => {
            // File not found: only warn if not a built-in
            if name != "dark" && name != "light" {
                notes.push(format!("theme \"{name}\" not found"));
            }
            None
        }
    };

    // If file provided, use dark base and apply file overrides
    let mut palette = if let Some(ref section) = file_section {
        let dark = Palette::dark();
        let base_entries: Vec<(String, Style)> = dark.syntax.entries;
        let mut warn_inner = Vec::new();
        let syntax = merge_syntax(base_entries, &section.syntax, &mut warn_inner);
        let mut p = Palette { syntax, ..dark };
        apply_ui_overrides(&mut p, &section.ui, &mut notes);
        notes.extend(warn_inner);
        p
    } else {
        base
    };

    // Also apply cfg.theme overrides on top
    let cfg_section = &cfg.theme;
    if !cfg_section.ui.is_empty() || !cfg_section.syntax.is_empty() {
        let base_entries: Vec<(String, Style)> = palette.syntax.entries;
        let mut w = Vec::new();
        palette.syntax = merge_syntax(base_entries, &cfg_section.syntax, &mut w);
        apply_ui_overrides(&mut palette, &cfg_section.ui, &mut notes);
        notes.extend(w);
    }

    (palette, notes)
}

pub fn resolve(cfg: &crate::config::Config) -> (Palette, Vec<String>) {
    let config_path = crate::config::config_path();
    let themes_dir = config_path.parent().unwrap_or(Path::new(".")).join("themes");
    resolve_in(cfg, &themes_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_color_names() {
        assert_eq!(parse_color("red").unwrap(), Color::Red);
        assert_eq!(parse_color("green").unwrap(), Color::Green);
        assert_eq!(parse_color("yellow").unwrap(), Color::Yellow);
        assert_eq!(parse_color("blue").unwrap(), Color::Blue);
        assert_eq!(parse_color("magenta").unwrap(), Color::Magenta);
        assert_eq!(parse_color("cyan").unwrap(), Color::Cyan);
        assert_eq!(parse_color("white").unwrap(), Color::White);
        assert_eq!(parse_color("black").unwrap(), Color::Black);
        
        assert_eq!(parse_color("bright-red").unwrap(), Color::LightRed);
        assert_eq!(parse_color("bright-yellow").unwrap(), Color::LightYellow);
        assert_eq!(parse_color("gray").unwrap(), Color::Gray);
        assert_eq!(parse_color("grey").unwrap(), Color::Gray);
        assert_eq!(parse_color("dark-gray").unwrap(), Color::DarkGray);
        assert_eq!(parse_color("reset").unwrap(), Color::Reset);
    }

    #[test]
    fn parse_color_hex_and_index() {
        assert_eq!(parse_color("#ff00af").unwrap(), Color::Rgb(255, 0, 175));
        assert_eq!(parse_color("240").unwrap(), Color::Indexed(240));
        assert_eq!(parse_color("0").unwrap(), Color::Indexed(0));
        assert_eq!(parse_color("255").unwrap(), Color::Indexed(255));
    }

    #[test]
    fn parse_color_junk_errs() {
        let err_text = parse_color("notacolor").unwrap_err();
        assert!(err_text.contains("notacolor"));
        
        let err_hex = parse_color("#gg0000").unwrap_err();
        assert!(err_hex.contains("#gg0000"));
        
        let err_256 = parse_color("256").unwrap_err();
        assert!(err_256.contains("256"));
        
        let err_neg = parse_color("-1").unwrap_err();
        assert!(err_neg.contains("-1"));
        
        let err_empty = parse_color("").unwrap_err();
        assert!(!err_empty.is_empty());
    }

    #[test]
    fn style_spec_apply() {
        use std::collections::BTreeMap;
        
        // Test: k = "blue" over base of fg DarkGray italic keeps italic, replaces fg
        let toml_blue = r#"k = "blue""#;
        let map: BTreeMap<String, StyleSpec> = toml::from_str(toml_blue).unwrap();
        let base = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
        let mut warns = Vec::new();
        let result = map.get("k").unwrap().apply(base, &mut warns);
        assert!(warns.is_empty());
        assert_eq!(result.fg, Some(Color::Blue));
        assert!(result.add_modifier.contains(Modifier::ITALIC));
        
        // Test: { italic = false } clears italic
        let toml_clear = r#"k = { italic = false }"#;
        let map: BTreeMap<String, StyleSpec> = toml::from_str(toml_clear).unwrap();
        let base = Style::default().add_modifier(Modifier::ITALIC);
        let mut warns = Vec::new();
        let result = map.get("k").unwrap().apply(base, &mut warns);
        assert!(warns.is_empty());
        assert!(!result.add_modifier.contains(Modifier::ITALIC));
        
        // Test: { fg = "red", bg = "#010203", bold = true } sets exactly those
        let toml_full = "k = { fg = \"red\", bg = \"#010203\", bold = true }";
        let map: BTreeMap<String, StyleSpec> = toml::from_str(toml_full).unwrap();
        let base = Style::default();
        let mut warns = Vec::new();
        let result = map.get("k").unwrap().apply(base, &mut warns);
        assert!(warns.is_empty());
        assert_eq!(result.fg, Some(Color::Red));
        assert_eq!(result.bg, Some(Color::Rgb(1, 2, 3)));
        assert!(result.add_modifier.contains(Modifier::BOLD));
        
        // Test: bad fg string pushes exactly one warning containing the key context
        let toml_bad = r#"mykey = { fg = "badcolor" }"#;
        let map: BTreeMap<String, StyleSpec> = toml::from_str(toml_bad).unwrap();
        let base = Style::default();
        let mut warns = Vec::new();
        let result = map.get("mykey").unwrap().apply(base, &mut warns);
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("badcolor"));
        assert_eq!(result, base);
    }

    // Helper: make a default Config with a given theme_name
    fn cfg_with_name(name: &str) -> crate::config::Config {
        let mut c = crate::config::Config::defaults();
        c.editor.theme_name = name.to_string();
        c
    }

    fn tmp_themes_dir() -> std::path::PathBuf {
        let pid = std::process::id();
        let dir = std::path::PathBuf::from(format!("/tmp/opencode/plat-themes-{pid}"));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn dark_syntax_table_pins_today() {
        let p = Palette::dark();
        assert_eq!(p.syntax.style_for_scope("keyword.function"), Some(Style::default().fg(Color::LightMagenta)));
        assert_eq!(p.syntax.style_for_scope("comment"), Some(Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)));
        assert_eq!(p.syntax.style_for_scope("function.method"), Some(Style::default().fg(Color::LightCyan)));
        assert_eq!(p.syntax.style_for_scope("variable"), None);
        assert_eq!(p.syntax.style_for_scope("punctuation.bracket"), None);
    }

    #[test]
    fn longest_prefix_wins_under_overrides() {
        let mut cfg = cfg_with_name("dark");
        cfg.theme.syntax.insert("function".to_string(), StyleSpec::Color("red".to_string()));
        let dir = tmp_themes_dir();
        let (p, _) = resolve_in(&cfg, &dir);
        // function.method longer prefix still wins for that scope
        assert_eq!(p.syntax.style_for_scope("function.method"), Some(Style::default().fg(Color::LightCyan)));
        // function scope now red
        assert_eq!(p.syntax.style_for_scope("function"), Some(Style::default().fg(Color::Red)));
    }

    #[test]
    fn per_key_merge_inherits_base_style() {
        let mut cfg = cfg_with_name("dark");
        cfg.theme.syntax.insert("comment".to_string(), StyleSpec::Color("blue".to_string()));
        let dir = tmp_themes_dir();
        let (p, _) = resolve_in(&cfg, &dir);
        let s = p.syntax.style_for_scope("comment").unwrap();
        assert_eq!(s.fg, Some(Color::Blue));
        // ITALIC preserved from dark base
        assert!(s.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn unknown_theme_name_falls_back_with_note() {
        let cfg = cfg_with_name("nope");
        let dir = tmp_themes_dir();
        let (p, notes) = resolve_in(&cfg, &dir);
        let dark = Palette::dark();
        // palette matches dark
        assert_eq!(p.syntax.style_for_scope("keyword"), dark.syntax.style_for_scope("keyword"));
        assert_eq!(p.selection.bg, dark.selection.bg);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("nope"));
    }

    #[test]
    fn file_shadows_builtin() {
        let pid = std::process::id();
        let themes_dir = std::path::PathBuf::from(format!("/tmp/opencode/plat-shadow-{pid}"));
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(themes_dir.join("light.toml"), "[syntax]\nkeyword = \"red\"\n").unwrap();
        let cfg = cfg_with_name("light");
        let (p, notes) = resolve_in(&cfg, &themes_dir);
        assert!(notes.is_empty(), "unexpected notes: {notes:?}");
        assert_eq!(p.syntax.style_for_scope("keyword"), Some(Style::default().fg(Color::Red)));
        // dark base for everything else
        assert_eq!(p.syntax.style_for_scope("comment"), Some(Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)));
    }

    #[test]
    fn bad_values_warn_not_fatal() {
        let pid = std::process::id();
        let themes_dir = std::path::PathBuf::from(format!("/tmp/opencode/plat-bad-{pid}"));
        std::fs::create_dir_all(&themes_dir).unwrap();
        std::fs::write(themes_dir.join("dark.toml"),
            "[ui]\nselection = \"nosuchcolor\"\nbogus_role = \"red\"\n").unwrap();
        let cfg = cfg_with_name("dark");
        let (p, notes) = resolve_in(&cfg, &themes_dir);
        // selection stays dark base bg Blue
        assert_eq!(p.selection.bg, Some(Color::Blue));
        // notes mention bogus_role and count 1 invalid color
        let has_bogus = notes.iter().any(|n| n.contains("bogus_role"));
        let has_invalid = notes.iter().any(|n| n.contains("1 invalid color"));
        assert!(has_bogus, "expected bogus_role note, got: {notes:?}");
        assert!(has_invalid, "expected invalid color count, got: {notes:?}");
    }

    #[test]
    fn parse_color_dark_aliases() {
        assert_eq!(parse_color("dark-cyan").unwrap(), Color::Cyan);
        assert_eq!(parse_color("dark-yellow").unwrap(), Color::Yellow);
    }

    #[test]
    fn light_palette_spot() {
        let p = Palette::light();
        assert_eq!(p.status_bar.fg, Some(Color::Black));
        assert_eq!(p.status_bar.bg, Some(Color::Gray));
        assert!(p.tab_active.add_modifier.contains(Modifier::BOLD));
        assert_eq!(p.selection.bg, Some(Color::Gray));
        // New overrides per spec
        assert_eq!(p.completion_sel.bg, Some(Color::Gray));
        assert_eq!(p.sidebar_sel.bg, Some(Color::Gray));
        assert_eq!(p.diag_warn.fg, Some(Color::Yellow));
        // Syntax overrides
        assert_eq!(p.syntax.style_for_scope("keyword").map(|s| s.fg), Some(Some(Color::Blue)));
        let comment = p.syntax.style_for_scope("comment").unwrap();
        assert_eq!(comment.fg, Some(Color::DarkGray));
        assert!(comment.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(p.syntax.style_for_scope("string").map(|s| s.fg), Some(Some(Color::Green)));
    }

    #[test]
    fn default_role_override() {
        // [theme.ui] default = base text fg/bg for the editor area
        assert_eq!(Palette::dark().default, Style::default(), "dark: default role must be a no-op");
        let cfg = crate::config::from_str("[theme.ui]\ndefault = { fg = \"#cdd6f4\", bg = \"#1e1e2e\" }\n").unwrap();
        let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
        assert_eq!(p.default.fg, Some(Color::Rgb(0xcd, 0xd6, 0xf4)));
        assert_eq!(p.default.bg, Some(Color::Rgb(0x1e, 0x1e, 0x2e)));
        assert!(notes.is_empty(), "default must be a known ui role, got: {notes:?}");
    }

    #[test]
    fn menu_bg_role() {
        // #40: dropdown (completion + code-action) background rectangle
        assert_eq!(Palette::dark().menu_bg.bg, Some(Color::DarkGray), "dark built-in: distinct from any default editor bg");
        assert_eq!(Palette::light().menu_bg.bg, Some(Color::Gray), "light built-in: distinct from a light editor bg");
        let cfg = crate::config::from_str("[theme.ui]\nmenu_bg = { bg = \"#123456\" }\n").unwrap();
        let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
        assert_eq!(p.menu_bg.bg, Some(Color::Rgb(0x12, 0x34, 0x56)), "bg override");
        assert!(notes.is_empty(), "menu_bg must be a known role, got: {notes:?}");
    }

    #[test]
    fn line_no_role() {
        assert_eq!(Palette::dark().line_no, Style::default(), "dark built-in: empty, numbers keep the base fg");
        assert_eq!(Palette::light().line_no, Style::default(), "light built-in: same");
        let cfg = crate::config::from_str("[theme.ui]\nline_no = { fg = \"#45475a\" }\n").unwrap();
        let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
        assert_eq!(p.line_no.fg, Some(Color::Rgb(69, 71, 90)), "fg override");
        assert!(notes.is_empty(), "line_no must be a known role, got: {notes:?}");
    }

    #[test]
    fn sidebar_dir_role() {
        use ratatui::style::Modifier;
        assert!(Palette::dark().sidebar_dir.add_modifier.contains(Modifier::BOLD), "built-in: dirs bold");
        let cfg = crate::config::from_str("[theme.ui]\nsidebar_dir = { fg = \"#89b4fa\" }\n").unwrap();
        let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
        assert_eq!(p.sidebar_dir.fg, Some(Color::Rgb(137, 180, 250)), "fg override");
        assert!(p.sidebar_dir.add_modifier.contains(Modifier::BOLD), "override merges onto the built-in");
        assert!(notes.is_empty(), "sidebar_dir must be a known role, got: {notes:?}");
    }

    #[test]
    fn current_line_no_role() {
        let s = Palette::dark().current_line_no;
        assert!(s.add_modifier.contains(Modifier::BOLD | Modifier::UNDERLINED), "dark built-in: BOLD+UNDERLINED");
        assert_eq!(s.fg, None, "dark built-in: no fg override");
        let l = Palette::light().current_line_no;
        assert!(l.add_modifier.contains(Modifier::BOLD | Modifier::UNDERLINED), "light inherits");
        let cfg = crate::config::from_str("[theme.ui]\ncurrent_line_no = { fg = \"red\" }\n").unwrap();
        let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
        assert_eq!(p.current_line_no.fg, Some(Color::Red), "fg override");
        assert!(p.current_line_no.add_modifier.contains(Modifier::UNDERLINED), "override merges onto built-in base");
        assert!(notes.is_empty(), "role name must be known, got: {notes:?}");
    }

    #[test]
    fn status_mode_role() {
        assert!(Palette::dark().status_mode.add_modifier.contains(Modifier::REVERSED), "dark built-in: REVERSE block");
        assert!(Palette::light().status_mode.add_modifier.contains(Modifier::REVERSED), "light inherits");
        let cfg = crate::config::from_str("[theme.ui]\nstatus_mode = { bold = true }\n").unwrap();
        let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
        assert!(p.status_mode.add_modifier.contains(Modifier::REVERSED), "override merges onto built-in");
        assert!(p.status_mode.add_modifier.contains(Modifier::BOLD));
        assert!(notes.is_empty(), "role name must be known, got: {notes:?}");
    }

    #[test]
    fn palette_sel_role() {
        assert_eq!(Palette::dark().palette_sel.bg, Some(Color::Blue), "dark: bg Blue");
        assert_eq!(Palette::light().palette_sel.bg, Some(Color::Gray), "light mirrors completion_sel: bg Gray");
        let cfg = crate::config::from_str("[theme.ui]\npalette_sel = { bg = \"#ff0000\" }\n").unwrap();
        let (p, notes) = resolve_in(&cfg, std::path::Path::new("/nonexistent-plat-themes"));
        assert_eq!(p.palette_sel.bg, Some(Color::Rgb(255,0,0)));
        assert!(notes.is_empty(), "palette_sel must be a known role, got {notes:?}");
    }
}
