/// Terminal capability detection for progressive rendering.
///
/// Detects terminal color support and dimensions to select the appropriate
/// card layout: Dumb → Basic → Extended → Full → TrueColor.
/// Width is re-queried on every render (never cached) so that resize events
/// are picked up immediately (e.g. `bop status --watch` in a Zellij pane).
use terminal_size::{terminal_size, Width};

/// Terminal capability level, ordered for comparison.
/// Higher levels are strictly more capable than lower ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TermLevel {
    /// TERM=dumb or unset; no color, ASCII only.
    Dumb,
    /// 8-color ANSI; ASCII borders.
    Basic,
    /// 16-color; box-drawing (single line); unicode blocks.
    Extended,
    /// 256-color; block shading ░▒▓; full BMP unicode.
    Full,
    /// 24-bit RGB; double-line box ╔═╗; playing-card tokens.
    TrueColor,
}

/// Detected terminal capabilities for a single render frame.
pub struct TermCaps {
    pub level: TermLevel,
    pub width: u16,
    /// Whether two-column layout is feasible (width ≥ 100).
    pub two_column: bool,
}

impl TermCaps {
    /// Detect terminal capabilities from environment and ioctl.
    ///
    /// Re-call on every render — width is never cached.
    pub fn detect() -> Self {
        let level = Self::detect_level();
        let width = terminal_size().map(|(Width(w), _)| w).unwrap_or(80);
        TermCaps {
            level,
            width,
            two_column: width >= 100,
        }
    }

    /// Classify terminal color/glyph support from environment variables.
    ///
    /// Detection order:
    /// 1. ZELLIJ env set → Full
    /// 2. COLORTERM = truecolor | 24bit → TrueColor
    /// 3. TERM contains "256color" → Full
    /// 4. TERM starts with xterm/screen/tmux/rxvt → Extended
    /// 5. TERM = "dumb" or empty → Dumb
    /// 6. Otherwise → Basic
    fn detect_level() -> TermLevel {
        let zellij = std::env::var("ZELLIJ").ok();
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        let term = std::env::var("TERM").unwrap_or_default();
        classify_level(zellij.as_deref(), &colorterm, &term)
    }
}

/// Pure classification logic — no env access, fully testable.
///
/// Detection order:
/// 1. `zellij` is Some → Full (Zellij always supports 256-color + unicode)
/// 2. `colorterm` = "truecolor" | "24bit" → TrueColor
/// 3. `term` contains "256color" → Full
/// 4. `term` starts with xterm/screen/tmux/rxvt → Extended
/// 5. `term` = "dumb" or empty → Dumb
/// 6. Otherwise → Basic
fn classify_level(zellij: Option<&str>, colorterm: &str, term: &str) -> TermLevel {
    // Zellij always supports Full (256-color + unicode).
    if zellij.is_some() {
        return TermLevel::Full;
    }

    if colorterm == "truecolor" || colorterm == "24bit" {
        return TermLevel::TrueColor;
    }

    if term.contains("256color") {
        return TermLevel::Full;
    }
    if term.starts_with("xterm")
        || term.starts_with("screen")
        || term.starts_with("tmux")
        || term.starts_with("rxvt")
    {
        return TermLevel::Extended;
    }
    if term == "dumb" || term.is_empty() {
        return TermLevel::Dumb;
    }

    TermLevel::Basic
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_level tests (pure, no env mutation) ─────────────────────────

    #[test]
    fn zellij_returns_full() {
        assert_eq!(classify_level(Some("0"), "", ""), TermLevel::Full);
    }

    #[test]
    fn colorterm_truecolor_returns_truecolor() {
        assert_eq!(classify_level(None, "truecolor", ""), TermLevel::TrueColor);
    }

    #[test]
    fn colorterm_24bit_returns_truecolor() {
        assert_eq!(classify_level(None, "24bit", ""), TermLevel::TrueColor);
    }

    #[test]
    fn term_256color_returns_full() {
        assert_eq!(classify_level(None, "", "xterm-256color"), TermLevel::Full);
    }

    #[test]
    fn term_xterm_returns_extended() {
        assert_eq!(classify_level(None, "", "xterm"), TermLevel::Extended);
    }

    #[test]
    fn term_screen_returns_extended() {
        assert_eq!(classify_level(None, "", "screen"), TermLevel::Extended);
    }

    #[test]
    fn term_tmux_returns_extended() {
        assert_eq!(classify_level(None, "", "tmux"), TermLevel::Extended);
    }

    #[test]
    fn term_rxvt_returns_extended() {
        assert_eq!(
            classify_level(None, "", "rxvt-unicode"),
            TermLevel::Extended
        );
    }

    #[test]
    fn term_dumb_returns_dumb() {
        assert_eq!(classify_level(None, "", "dumb"), TermLevel::Dumb);
    }

    #[test]
    fn term_empty_returns_dumb() {
        assert_eq!(classify_level(None, "", ""), TermLevel::Dumb);
    }

    #[test]
    fn term_unknown_returns_basic() {
        assert_eq!(classify_level(None, "", "vt100"), TermLevel::Basic);
    }

    #[test]
    fn zellij_takes_priority_over_colorterm() {
        // ZELLIJ is checked first, so even with truecolor COLORTERM, result is Full.
        assert_eq!(
            classify_level(Some("0"), "truecolor", "xterm-256color"),
            TermLevel::Full
        );
    }

    #[test]
    fn colorterm_takes_priority_over_term() {
        // COLORTERM truecolor beats TERM xterm (which would be Extended).
        assert_eq!(
            classify_level(None, "truecolor", "xterm"),
            TermLevel::TrueColor
        );
    }

    // ── TermLevel ordering ──────────────────────────────────────────────────

    #[test]
    fn level_ordering() {
        assert!(TermLevel::Dumb < TermLevel::Basic);
        assert!(TermLevel::Basic < TermLevel::Extended);
        assert!(TermLevel::Extended < TermLevel::Full);
        assert!(TermLevel::Full < TermLevel::TrueColor);
    }

    // ── TermCaps struct tests ───────────────────────────────────────────────

    #[test]
    fn two_column_below_threshold() {
        let caps = TermCaps {
            level: TermLevel::Full,
            width: 80,
            two_column: 80 >= 100,
        };
        assert!(!caps.two_column);
    }

    #[test]
    fn two_column_above_threshold() {
        let caps = TermCaps {
            level: TermLevel::Full,
            width: 120,
            two_column: 120 >= 100,
        };
        assert!(caps.two_column);
    }

    #[test]
    fn two_column_exact_threshold() {
        let caps = TermCaps {
            level: TermLevel::Full,
            width: 100,
            two_column: 100 >= 100,
        };
        assert!(caps.two_column);
    }

    #[test]
    fn detect_returns_valid_caps() {
        let caps = TermCaps::detect();
        // Width should be positive
        assert!(caps.width > 0);
        // two_column must be consistent with width
        assert_eq!(caps.two_column, caps.width >= 100);
    }
}
