//! Symbol data types and LSP parsing for workspace symbols feature.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;

/// LSP SymbolKind values (0–26); see LSP spec
pub const SYMBOL_KIND_FUNCTION: i32 = 12;
pub const SYMBOL_KIND_CLASS: i32 = 5;
pub const SYMBOL_KIND_STRUCT: i32 = 23;
pub const SYMBOL_KIND_ENUM: i32 = 11;
pub const SYMBOL_KIND_VARIABLE: i32 = 13;
pub const SYMBOL_KIND_METHOD: i32 = 6;
pub const SYMBOL_KIND_PROPERTY: i32 = 8;
pub const SYMBOL_KIND_INTERFACE: i32 = 11;
pub const SYMBOL_KIND_MODULE: i32 = 2;
pub const SYMBOL_KIND_CONSTANT: i32 = 14;

/// Parsed LSP symbol with location info for jumping
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LspSymbol {
    pub name: String,
    pub kind: i32,
    pub range: (u32, u32, u32, u32), // (start_line, start_col, end_line, end_col)
    pub detail: Option<String>,      // Type signature
    pub container: Option<String>,   // Parent scope (workspace symbols only)
}

impl LspSymbol {
    /// Extract just the location info (URI + range) for jumping
    pub fn location(&self, uri: &str) -> (String, u32, u32) {
        (uri.to_string(), self.range.0, self.range.1)
    }
}

/// Parse textDocument/documentSymbol response
pub fn parse_file_symbols(json: Value) -> Result<Vec<LspSymbol>, Box<dyn Error>> {
    let array = json.as_array().ok_or("Expected array")?;
    let mut symbols = Vec::new();
    
    for item in array {
        let name = item["name"].as_str().ok_or("Missing name")?.to_string();
        let kind = item["kind"].as_i64().ok_or("Missing kind")? as i32;
        
        let range = {
            let start_line = item["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
            let start_char = item["range"]["start"]["character"].as_u64().unwrap_or(0) as u32;
            let end_line = item["range"]["end"]["line"].as_u64().unwrap_or(0) as u32;
            let end_char = item["range"]["end"]["character"].as_u64().unwrap_or(0) as u32;
            (start_line, start_char, end_line, end_char)
        };
        
        let detail = item["detail"].as_str().map(|s| s.to_string());
        
        symbols.push(LspSymbol {
            name,
            kind,
            range,
            detail,
            container: None, // file symbols don't have container
        });
    }
    
    Ok(symbols)
}

/// Parse workspace/symbol response
pub fn parse_workspace_symbols(json: Value) -> Result<Vec<LspSymbol>, Box<dyn Error>> {
    let array = json.as_array().ok_or("Expected array")?;
    let mut symbols = Vec::new();
    
    for item in array {
        let name = item["name"].as_str().ok_or("Missing name")?.to_string();
        let kind = item["kind"].as_i64().ok_or("Missing kind")? as i32;
        
        let range = {
            let start_line = item["location"]["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
            let start_char = item["location"]["range"]["start"]["character"].as_u64().unwrap_or(0) as u32;
            let end_line = item["location"]["range"]["end"]["line"].as_u64().unwrap_or(0) as u32;
            let end_char = item["location"]["range"]["end"]["character"].as_u64().unwrap_or(0) as u32;
            (start_line, start_char, end_line, end_char)
        };
        
        let container = item["containerName"].as_str().map(|s| s.to_string());
        
        symbols.push(LspSymbol {
            name,
            kind,
            range,
            detail: None, // workspace symbols don't include detail
            container,
        });
    }
    
    Ok(symbols)
}

/// Map LSP SymbolKind to Unicode icon for display
pub fn symbol_kind_icon(kind: i32) -> &'static str {
    match kind {
        12 | 6 => "ƒ",  // Function, Method
        5 | 23 => "Ⓣ", // Class, Struct
        11 => "♦",      // Enum
        13 => "Ϸ",      // Variable
        8 => "Ⓟ",       // Property
        9 => "◇",       // Interface
        2 => "⊕",       // Module
        14 => "≡",      // Constant
        _ => "•",       // Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_symbol_location() {
        let symbol = LspSymbol {
            name: "main".to_string(),
            kind: 12,
            range: (5, 3, 10, 0),
            detail: None,
            container: None,
        };
        
        let (uri, line, col) = symbol.location("file:///test.rs");
        assert_eq!(uri, "file:///test.rs");
        assert_eq!(line, 5);
        assert_eq!(col, 3);
    }
}
