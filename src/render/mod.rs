//! Widget composition root.
//!
//! Reads model snapshots only — no terminal I/O, no crossterm events, no
//! network or file I/O, no model mutation.

pub mod graphs;
pub mod panels;
pub mod skyplot;
pub mod table;

use ratatui::style::Color;

use crate::model::ServerState;

/// Stable per-server display color, assigned by positional index.
///
/// Palette (indices 0–5): Cyan, Yellow, Magenta, Green, Red, Blue.
/// Indices ≥ 6 wrap via `index % 6` so no call ever panics or returns an
/// off-palette color.  The count is capped at 6 upstream, so the wrap branch
/// is a defensive guarantee rather than a normal code path.
pub fn server_color(index: usize) -> Color {
    match index {
        0 => Color::Cyan,
        1 => Color::Yellow,
        2 => Color::Magenta,
        3 => Color::Green,
        4 => Color::Red,
        5 => Color::Blue,
        _ => server_color(index % 6),
    }
}

/// Format a server's status for display in the UI.
pub fn format_status(server: &ServerState) -> &'static str {
    use crate::model::ServerStatus;
    match server.status() {
        ServerStatus::Connecting => "connecting",
        ServerStatus::Ok => "ok",
        ServerStatus::Stale => "stale",
        ServerStatus::Unreachable => "unreachable",
    }
}

/// Format an `Option<f64>` for display, showing "n/a" when `None`.
pub fn fmt_opt_f64(v: Option<f64>, precision: usize) -> String {
    match v {
        Some(f) => format!("{f:.precision$}"),
        None => "n/a".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_color_palette() {
        assert_eq!(server_color(0), Color::Cyan);
        assert_eq!(server_color(1), Color::Yellow);
        assert_eq!(server_color(2), Color::Magenta);
        assert_eq!(server_color(3), Color::Green);
        assert_eq!(server_color(4), Color::Red);
        assert_eq!(server_color(5), Color::Blue);
        // Out-of-range wraps at 6.
        assert_eq!(server_color(6), Color::Cyan);
        assert_eq!(server_color(7), Color::Yellow);
    }

    #[test]
    fn fmt_opt_f64_some() {
        let s = fmt_opt_f64(Some(0.000123), 6);
        assert_eq!(s, "0.000123");
    }

    #[test]
    fn fmt_opt_f64_none() {
        assert_eq!(fmt_opt_f64(None, 3), "n/a");
    }
}
