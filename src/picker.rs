use crate::tree::TreeItem;

#[derive(Clone, Debug)]
pub struct FuzzyResult {
    pub item: TreeItem,
    pub score: u32,
}

pub struct FuzzyMatcher {
    pub items: Vec<TreeItem>,
}

impl FuzzyMatcher {
    pub fn new(items: Vec<TreeItem>) -> Self {
        FuzzyMatcher { items }
    }

    pub fn search(&self, term: &str) -> Vec<FuzzyResult> {
        if term.is_empty() {
            let mut results: Vec<_> = self
                .items
                .iter()
                .map(|item| FuzzyResult { item: item.clone(), score: 100 })
                .collect();
            results.sort_by_key(|r| r.item.path.clone());
            return results;
        }

        let term = term.to_lowercase();
        let mut results = Vec::new();
        for item in &self.items {
            // only files are searchable in the picker
            if item.is_dir {
                continue;
            }
            let filename = item
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let rel = item
                .path
                .to_string_lossy()
                .to_lowercase()
                .replace('\\', "/");

            let score = if filename == term {
                1000
            } else if filename.starts_with(&term) {
                900
            } else if filename.contains(&term) {
                800
            } else if rel.contains(&term) {
                600
            } else {
                0
            };

            if score > 0 {
                results.push(FuzzyResult { item: item.clone(), score });
            }
        }

        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.path.cmp(&b.item.path)));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::TreeItem;
    use std::path::PathBuf;

    fn matcher() -> FuzzyMatcher {
        FuzzyMatcher {
            items: vec![
                TreeItem { path: PathBuf::from("src/main.rs"), is_dir: false, depth: 1 },
                TreeItem { path: PathBuf::from("src/app.rs"), is_dir: false, depth: 1 },
                TreeItem { path: PathBuf::from("src/render.rs"), is_dir: false, depth: 1 },
                TreeItem { path: PathBuf::from("tests/flow.rs"), is_dir: false, depth: 1 },
            ],
        }
    }

    #[test]
    fn search_empty_term_returns_all_items() {
        let results = matcher().search("");
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn search_exact_filename_match_highest_score() {
        let results = matcher().search("main");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item.path.to_string_lossy(), "src/main.rs");
    }

    #[test]
    fn search_substring_matches_multiple() {
        let results = matcher().search("app");
        assert!(results.iter().any(|r| r.item.path.to_string_lossy().contains("app")));
    }

    #[test]
    fn search_path_component_match() {
        let results = matcher().search("src/r");
        assert!(results.iter().any(|r| r.item.path.to_string_lossy().contains("render")));
    }

    #[test]
    fn search_results_sorted_by_score_descending() {
        let results = matcher().search("r");
        assert!(results[0].score >= results.get(1).map(|r| r.score).unwrap_or(0));
    }
}
