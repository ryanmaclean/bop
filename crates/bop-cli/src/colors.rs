/// Shared color constants for bop CLI output.
///
/// Provides both hex colors (for HTML/SVG) and ANSI terminal colors
/// for consistent state visualization across commands.
/// Hex color code for a given state (for HTML/SVG rendering).
pub fn state_color(state: &str) -> &'static str {
    match state {
        "pending" => "#3A5A8A",
        "running" => "#B8690F",
        "done" => "#1E8A45",
        "failed" => "#C43030",
        "merged" => "#6B3DB8",
        _ => "#555",
    }
}

/// ANSI 256-color foreground code for a given state.
pub fn state_ansi(state: &str) -> &'static str {
    match state {
        "pending" => "\x1b[38;5;67m",  // steel blue
        "running" => "\x1b[38;5;172m", // amber
        "done" => "\x1b[38;5;71m",     // green
        "failed" => "\x1b[38;5;160m",  // red
        "merged" => "\x1b[38;5;134m",  // violet
        _ => "\x1b[38;5;240m",         // gray
    }
}

/// ANSI 256-color background code for a given state (for bar fill).
pub fn state_ansi_bg(state: &str) -> &'static str {
    match state {
        "pending" => "\x1b[48;5;67m",
        "running" => "\x1b[48;5;172m",
        "done" => "\x1b[48;5;71m",
        "failed" => "\x1b[48;5;160m",
        "merged" => "\x1b[48;5;134m",
        _ => "\x1b[48;5;240m",
    }
}

/// ANSI control sequences for text styling.
pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
