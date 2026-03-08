/// Nucleo-powered fuzzy filter for the kanban board.
///
/// When the user presses `/` in Normal mode, the TUI enters `Mode::Filter`.
/// Each keystroke updates the nucleo pattern and filters all cards across
/// all columns by matching `card.id + " " + card.title`. Columns with no
/// matches after filtering collapse to narrow dividers. Matched characters
/// are highlighted in card titles with `Color::Yellow` + `BOLD` using
/// nucleo's match indices.
///
/// - `Esc` clears the filter and restores full columns.
/// - `Enter` confirms the filter (stays in Normal with filter active).
use nucleo::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo::{Matcher, Utf32String};

/// Result of matching a single card against the current filter pattern.
#[derive(Debug, Clone)]
pub struct FilterMatch {
    /// Indices into the card title string that matched (sorted, deduplicated).
    /// Used for highlight rendering in the kanban widget.
    pub title_indices: Vec<u32>,
}

/// Shared filter state held by [`App`](crate::ui::app::App).
///
/// Created when entering Filter mode, updated on each keystroke, and
/// dropped when the filter is cleared. The `Matcher` is reused across
/// keystrokes to avoid repeated allocation.
pub struct FilterState {
    /// The current query string being typed by the user.
    pub query: String,
    /// Reusable nucleo matcher instance.
    matcher: Matcher,
}

impl FilterState {
    /// Create a new empty filter state with a default matcher.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            matcher: Matcher::new(nucleo::Config::DEFAULT),
        }
    }

    /// Test whether a card matches the current query.
    ///
    /// Matches against `"{id} {title}"` — the combined haystack gives
    /// users flexibility to filter by either card ID or title text.
    /// Returns `None` if no match; returns `Some(FilterMatch)` with
    /// highlighted indices into the *title* portion on match.
    ///
    /// When the query is empty, all cards match (with no highlights).
    pub fn matches(&mut self, card_id: &str, card_title: &str) -> Option<FilterMatch> {
        if self.query.is_empty() {
            return Some(FilterMatch {
                title_indices: Vec::new(),
            });
        }

        let pattern = Pattern::new(
            &self.query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );

        // Build combined haystack: "id title"
        let haystack_str = format!("{} {}", card_id, card_title);
        let haystack = Utf32String::from(haystack_str.as_str());

        // Check if the pattern matches the combined haystack.
        let mut indices: Vec<u32> = Vec::new();
        let score = pattern.indices(haystack.slice(..), &mut self.matcher, &mut indices);

        score?;

        indices.sort_unstable();
        indices.dedup();

        // Convert combined-haystack indices to title-only indices.
        // The title starts at byte offset (id.len() + 1) in the combined string,
        // but nucleo indices are char indices, so we need the char offset.
        let id_char_len = card_id.chars().count() as u32;
        let title_offset = id_char_len + 1; // +1 for the space separator

        let title_indices: Vec<u32> = indices
            .into_iter()
            .filter_map(|idx| {
                if idx >= title_offset {
                    Some(idx - title_offset)
                } else {
                    None
                }
            })
            .collect();

        Some(FilterMatch { title_indices })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_all() {
        let mut state = FilterState::new();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_some());
        assert!(result.unwrap().title_indices.is_empty());
    }

    #[test]
    fn query_matches_title() {
        let mut state = FilterState::new();
        state.query = "feat".into();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_some());
        // "feat" should match within "Feature" — indices should be non-empty.
        let m = result.unwrap();
        assert!(!m.title_indices.is_empty());
    }

    #[test]
    fn query_matches_id() {
        let mut state = FilterState::new();
        state.query = "card-1".into();
        let result = state.matches("card-1", "My Feature");
        // Should match (against the id portion).
        assert!(result.is_some());
    }

    #[test]
    fn query_no_match() {
        let mut state = FilterState::new();
        state.query = "zzzzz".into();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_none());
    }

    #[test]
    fn fuzzy_match_indices_are_sorted() {
        let mut state = FilterState::new();
        state.query = "mf".into();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_some());
        let m = result.unwrap();
        // Indices should be sorted.
        for window in m.title_indices.windows(2) {
            assert!(window[0] <= window[1]);
        }
    }

    #[test]
    fn case_insensitive_match() {
        let mut state = FilterState::new();
        state.query = "my feature".into();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_some());
    }
}
