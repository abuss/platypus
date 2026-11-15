
// Task 2: File Symbols Infrastructure Tests

#[test]
fn test_file_symbols_sidebar_mode_exists() {
    use plat::sidebar::SidebarMode;
    
    // Just verify the variant exists
    let _mode = SidebarMode::FileSymbols;
}

#[test]
fn test_file_symbol_state_new() {
    use plat::sidebar::FileSymbolState;
    
    let state = FileSymbolState::new();
    assert_eq!(state.symbols.len(), 0);
    assert_eq!(state.items.len(), 0);
    assert_eq!(state.cursor, 0);
    assert!(!state.truncated);
    assert_eq!(state.cached_doc_uri, "");
}

#[test]
fn test_file_symbol_state_selected_symbol_none() {
    use plat::sidebar::FileSymbolState;
    
    let state = FileSymbolState::new();
    assert_eq!(state.selected_symbol(), None);
}

#[test]
fn test_file_symbol_state_selected_symbol_some() {
    use plat::sidebar::FileSymbolState;
    use plat::symbols::LspSymbol;
    
    let mut state = FileSymbolState::new();
    let symbol = LspSymbol {
        name: "main".to_string(),
        kind: 12,
        range: (0, 0, 10, 0),
        detail: None,
        container: None,
        uri: None,
    };
    state.symbols.push(symbol.clone());
    state.cursor = 0;
    
    assert_eq!(state.selected_symbol().unwrap().name, "main");
}
