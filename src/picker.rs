use crate::tree::TreeItem;

#[derive(Clone, Debug)]
pub struct FuzzyResult {
    pub item: TreeItem,
    pub score: u32,
}

pub struct FuzzyMatcher {
    pub items: Vec<TreeItem>,
}

pub(crate) fn subsequence_score(hay: &str, term: &str) -> Option<u32> {
    subsequence_match(hay, term).map(|(score, _)| score)
}

/// Same scoring as `subsequence_score`, but also returns the byte offsets of
/// the matched characters (for highlight underlines in the sidebar).
pub(crate) fn subsequence_match(hay: &str, term: &str) -> Option<(u32, Vec<usize>)> {
    if term.is_empty() {
        return Some((0, Vec::new()));
    }

    let hay_bytes = hay.as_bytes();
    let term_bytes = term.as_bytes();
    let mut hay_idx = 0;
    let mut score = 0u32;
    let mut prev_matched_at = 0isize; // signed to handle first match
    let mut hits: Vec<usize> = Vec::new();

    for &term_char in term_bytes {
        let mut found = false;
        while hay_idx < hay_bytes.len() {
            if hay_bytes[hay_idx] == term_char {
                // Determine if this is a word boundary (start or after /, _, ., -)
                let is_word_start = hay_idx == 0 || 
                    matches!(hay_bytes[hay_idx - 1], b'/' | b'_' | b'.' | b'-');
                
                let bonus = if is_word_start { 8 } else { 2 };
                score += bonus;
                
                // Gap penalty: count chars between previous match and this one (excluding the matched char)
                if prev_matched_at >= 0 {
                    let gap = (hay_idx as isize - prev_matched_at - 1) as u32;
                    score = score.saturating_sub(gap);
                }
                
                prev_matched_at = hay_idx as isize;
                hits.push(hay_idx);
                hay_idx += 1;
                found = true;
                break;
            }
            hay_idx += 1;
        }

        if !found {
            return None; // incomplete subsequence
        }
    }

    // Length tiebreak: penalize longer hay relative to term
    let len_diff = (hay.len() as u32).saturating_sub(term.len() as u32) / 8;
    score = score.saturating_sub(len_diff);

    Some((score.max(1), hits))
}

impl FuzzyMatcher {
    pub fn new(items: Vec<TreeItem>) -> Self {
        FuzzyMatcher { items }
    }

    /// Scored matches as (item index, score, name-hit byte offsets). The
    /// sidebar filters per keystroke through this: returning INDICES avoids
    /// cloning every kept TreeItem (perf pass 2026-11-05). Hits come from the
    /// lowercased name, so underlines survive case-insensitive matches too.
    pub fn search_indexed(&self, term: &str) -> Vec<(usize, u32, Vec<usize>)> {
        if term.is_empty() {
            let mut results: Vec<(usize, u32, Vec<usize>)> =
                (0..self.items.len()).map(|i| (i, 100, Vec::new())).collect();
            results.sort_by(|a, b| self.items[a.0].path.cmp(&self.items[b.0].path));
            return results;
        }

        let term = term.to_lowercase();
        let mut results: Vec<(usize, u32, Vec<usize>)> = Vec::new();
        for (idx, item) in self.items.iter().enumerate() {
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

            let name_hits = || subsequence_match(&filename, &term).map(|(_, h)| h).unwrap_or_default();
            let score = if filename == term {
                1000
            } else if filename.starts_with(&term) {
                900
            } else if filename.contains(&term) {
                800
            } else if rel.contains(&term) {
                600
            } else if let Some(subseq_score) = subsequence_score(&filename, &term) {
                // filename subsequence match
                subseq_score
            } else if let Some(subseq_score) = subsequence_score(&rel, &term) {
                // path subsequence match, divided by 4
                subseq_score / 4
            } else {
                0
            };

            if score > 0 {
                results.push((idx, score, name_hits()));
            }
        }

        results.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| self.items[a.0].path.cmp(&self.items[b.0].path))
        });
        results
    }

    pub fn search(&self, term: &str) -> Vec<FuzzyResult> {
        self.search_indexed(term)
            .into_iter()
            .map(|(i, score, _)| FuzzyResult { item: self.items[i].clone(), score })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::TreeItem;
    use std::path::PathBuf;

    fn item(path: &str) -> TreeItem {
        TreeItem { path: PathBuf::from(path), is_dir: false, depth: 1 }
    }

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

    #[test]
    fn subsequence_ranks_fltr_over_a_far_path_match() {
        let m = FuzzyMatcher { items: vec![
            item("src/filter.rs"), item("src/lorem/flitster.rs"), item("docs/report_flitster_long_name.md") ] };
        let r = m.search("fltr");
        assert_eq!(r[0].item.path.to_string_lossy(), "src/filter.rs", "gap-penalty + shorter-name wins: {r:?}");
    }

    #[test]
    fn word_start_beats_mid_word() {
        let m = FuzzyMatcher { items: vec![item("src/my_filter.rs"), item("src/xfilter.rs")] };
        assert_eq!(m.search("fil")[0].item.path.to_string_lossy(), "src/my_filter.rs");
    }

    #[test]
    fn non_subsequence_still_scores_zero() {
        let m = FuzzyMatcher { items: vec![item("src/render.rs")] };
        assert!(m.search("zq").is_empty());
    }

    #[test]
    fn subsequence_match_returns_byte_positions() {
        let (score, hits) = subsequence_match("app.rs", "a.").unwrap();
        assert!(score > 0);
        assert_eq!(hits, vec![0, 3], "matched bytes at a(0) and .(3)");
        assert!(subsequence_match("app.rs", "z").is_none());
        // the classic score path must stay identical
        assert_eq!(Some(score), subsequence_score("app.rs", "a."));
    }
}
