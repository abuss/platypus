// Quick test to verify Super-\ normalization
use crossterm::event::{KeyCode, KeyModifiers};

fn normalize(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    match code {
        KeyCode::Char(c)
            if c.is_ascii_alphabetic()
                && mods.contains(KeyModifiers::CONTROL)
                && !mods.contains(KeyModifiers::ALT) =>
        {
            (KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::CONTROL)
        }
        KeyCode::Char(c)
            if c.is_ascii_uppercase()
                && (mods == KeyModifiers::SHIFT || mods == KeyModifiers::NONE) =>
        {
            (KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::SHIFT)
        }
        _ => (code, mods),
    }
}

fn main() {
    let backslash = KeyCode::Char('\\');
    
    // Test normalization of different modifier combinations
    println!("Normalizing backslash with different modifiers:");
    println!("Ctrl-\\:   {:?}", normalize(backslash, KeyModifiers::CONTROL));
    println!("Super-\\:  {:?}", normalize(backslash, KeyModifiers::SUPER));
    println!("Plain-\\:  {:?}", normalize(backslash, KeyModifiers::NONE));
    
    // Both should return the key as-is (not in the alphabetic rules)
    assert_eq!(
        normalize(backslash, KeyModifiers::SUPER),
        (backslash, KeyModifiers::SUPER),
        "Super-\ should pass through unchanged"
    );
    
    assert_eq!(
        normalize(backslash, KeyModifiers::CONTROL),
        (backslash, KeyModifiers::CONTROL),
        "Ctrl-\ should pass through unchanged"
    );
    
    println!("\nBoth normalize correctly!");
}
