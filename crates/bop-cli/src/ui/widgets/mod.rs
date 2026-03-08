/// TUI widget modules for the kanban board.
///
/// Each widget handles rendering for one zone or overlay of the three-zone
/// layout (header, body, footer). The kanban widget is the primary body
/// view; other widgets (logtail, filter, newcard) will be added
/// in subsequent phases.
pub mod action;
pub mod detail;
pub mod filter;
pub mod footer;
pub mod header;
pub mod kanban;
pub mod logtail;
pub mod newcard;

pub use detail::render_detail;
pub use footer::render_footer;
pub use header::render_header;
pub use kanban::render_kanban;
pub use logtail::render_logtail;
