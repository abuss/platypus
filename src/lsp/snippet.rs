/// Snippet placeholder stripping per LSP spec (flat grammar, no nesting needed).
/// Removes `$0`, `$1`, `${1}`, keeps inner text for `${1:text}`.
/// Handles `\$` → `$` and `\\` → `\` escapes.
///
/// ponytail: nested `${1:${2:inner}}` not required by spec; flat grammar covers
/// real-world snippet servers. Add recursive descent if a server ever emits nesting.
pub fn strip_snippet(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                // escape: \$ → $, \\ → \, anything else: pass both through
                match chars[i + 1] {
                    '$' => { out.push('$'); i += 2; }
                    '\\' => { out.push('\\'); i += 2; }
                    c => { out.push('\\'); out.push(c); i += 2; }
                }
            }
            '$' => {
                i += 1;
                if i < chars.len() && chars[i] == '{' {
                    // ${...} form: skip digits, colon, collect text until '}'
                    i += 1; // skip '{'
                    // skip digits (tabstop index)
                    while i < chars.len() && chars[i].is_ascii_digit() { i += 1; }
                    if i < chars.len() && chars[i] == ':' {
                        i += 1; // skip ':'
                        // collect placeholder text until matching '}'
                        while i < chars.len() && chars[i] != '}' {
                            out.push(chars[i]);
                            i += 1;
                        }
                    }
                    if i < chars.len() && chars[i] == '}' { i += 1; } // skip '}'
                } else {
                    // $N or $identifier form: skip digits/word chars
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i].is_ascii_alphabetic() || chars[i] == '_') {
                        i += 1;
                    }
                    // emit nothing (placeholder removed)
                }
            }
            c => { out.push(c); i += 1; }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_plain_text_unchanged() {
        assert_eq!(strip_snippet("hello"), "hello");
    }

    #[test]
    fn strip_simple_tab_stop() {
        assert_eq!(strip_snippet("fn $1()"), "fn ()");
        assert_eq!(strip_snippet("$0"), "");
    }

    #[test]
    fn strip_braced_no_placeholder() {
        assert_eq!(strip_snippet("${1}"), "");
        assert_eq!(strip_snippet("fn ${1}()"), "fn ()");
    }

    #[test]
    fn strip_braced_with_placeholder_text() {
        assert_eq!(strip_snippet("beta_x(${1:arg})"), "beta_x(arg)");
        assert_eq!(strip_snippet("beta_x(${1:arg})$2"), "beta_x(arg)");
    }

    #[test]
    fn strip_escaped_dollar() {
        assert_eq!(strip_snippet("\\$n"), "$n");
        assert_eq!(strip_snippet("price: \\$10"), "price: $10");
    }

    #[test]
    fn strip_escaped_backslash() {
        assert_eq!(strip_snippet("a\\\\b"), "a\\b");
    }

    #[test]
    fn strip_var_form() {
        // $TM_FILENAME etc are variable-form, stripped same as tab stops
        assert_eq!(strip_snippet("$var1"), "");
        assert_eq!(strip_snippet("file $TM_FILENAME end"), "file  end");
    }
}
