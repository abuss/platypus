use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

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
        "gray" | "grey" => return Ok(Color::Gray),
        "dark-gray" | "dark-grey" => return Ok(Color::DarkGray),
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

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum StyleSpec {
    Color(String),
    Index(i64),
    Full(StyleTable),
}

#[derive(Clone, Debug, Deserialize, Default)]
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
}
