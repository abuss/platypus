use std::path::Path;

/// Phase 1 simple .gitignore matching: `*.ext`, `dir/`, plain names.
/// Negation (`!rule`) and `**` globs deferred to Phase 1.1 (the `ignore` crate).
#[derive(Default)]
pub struct GitignoreRules {
    pub rules: Vec<String>,
}

impl GitignoreRules {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let rules = content
                    .lines()
                    .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
                    .map(|line| line.trim().to_string())
                    .collect();
                Ok(GitignoreRules { rules })
            }
            Err(_) => Ok(GitignoreRules { rules: vec![] }),
        }
    }

    pub fn matches(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        self.rules.iter().any(|rule| match_rule(&path_str, rule))
    }
}

fn match_rule(path_str: &str, rule: &str) -> bool {
    if rule.ends_with('/') {
        // directory pattern: match "dir" or anything under "dir/"
        let dir = &rule[..rule.len() - 1];
        path_str == dir || path_str.starts_with(&format!("{dir}/"))
    } else if rule.starts_with('*') {
        // extension pattern: *.ext
        path_str.ends_with(&rule[1..])
    } else {
        // literal: filename or any path component equals the rule
        path_str == rule
            || Path::new(path_str)
                .file_name()
                .and_then(|n| n.to_str())
                == Some(rule)
            || path_str.contains(&format!("/{rule}/"))
    }
}

pub fn default_ignores() -> Vec<&'static str> {
    vec![".git", "node_modules", "target", "__pycache__", ".egg-info"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn default_ignores_includes_git() {
        let defaults = default_ignores();
        assert!(defaults.contains(&".git"));
    }

    #[test]
    fn simple_glob_pattern_matches() {
        // "*.o" should match "file.o"
        let rules = GitignoreRules {
            rules: vec!["*.o".to_string()],
        };
        assert!(rules.matches(Path::new("file.o")));
        assert!(!rules.matches(Path::new("file.rs")));
    }

    #[test]
    fn directory_pattern_matches_dir_contents() {
        // "build/" should match "build" and "build/output.txt"
        let rules = GitignoreRules {
            rules: vec!["build/".to_string()],
        };
        assert!(rules.matches(Path::new("build")));
        assert!(rules.matches(Path::new("build/output.txt")));
    }

    #[test]
    fn from_file_parses_lines_ignoring_comments() {
        // temp file with "# comment\n*.log\n\n" -> one rule "*.log"
        let dir = std::env::temp_dir().join(format!(
            "plat-gitignore-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".gitignore");
        std::fs::write(&path, "# comment\n*.log\n\n").unwrap();

        let rules = GitignoreRules::from_file(&path).unwrap();
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0], "*.log");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn from_nonexistent_file_returns_empty_rules() {
        let rules = GitignoreRules::from_file(Path::new("/nonexistent")).unwrap_or_default();
        assert!(rules.rules.is_empty());
    }
}
