// Lightweight ID filter for the kanban board.
//
// When the user presses `/` in Normal mode, the TUI enters `Mode::Filter`.
// Each keystroke updates a case-insensitive substring filter on `card.id`.
// Columns with no matches after filtering collapse to narrow dividers.
//
// - `Esc` clears the filter and restores full columns.
// - `Enter` confirms the filter (stays in Normal with filter active).

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
/// dropped when the filter is cleared.
pub struct FilterState {
    /// The current query string being typed by the user.
    pub query: String,
}

impl FilterState {
    /// Create a new empty filter state.
    pub fn new() -> Self {
        Self {
            query: String::new(),
        }
    }

    /// Test whether a card matches the current query.
    ///
    /// Returns `None` if no match; returns `Some(FilterMatch)` on match.
    /// When the query is empty, all cards match.
    pub fn matches(&mut self, card_id: &str, _card_title: &str) -> Option<FilterMatch> {
        if self.query.is_empty() {
            return Some(FilterMatch {
                title_indices: Vec::new(),
            });
        }

        if card_id
            .to_ascii_lowercase()
            .contains(&self.query.to_ascii_lowercase())
        {
            Some(FilterMatch {
                title_indices: Vec::new(),
            })
        } else {
            None
        }
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
    fn query_does_not_match_title_only() {
        let mut state = FilterState::new();
        state.query = "feat".into();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_none());
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
    fn substring_match_has_no_title_highlights() {
        let mut state = FilterState::new();
        state.query = "card".into();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_some());
        let m = result.unwrap();
        assert!(m.title_indices.is_empty());
    }

    #[test]
    fn case_insensitive_match() {
        let mut state = FilterState::new();
        state.query = "CARD-1".into();
        let result = state.matches("card-1", "My Feature");
        assert!(result.is_some());
    }
}
