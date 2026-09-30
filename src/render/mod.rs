//! Widget composition root.
//!
//! Reads model snapshots only — no terminal I/O, no crossterm events, no
//! network or file I/O, no model mutation.

pub mod graphs;
pub mod panels;
pub mod table;

use ratatui::style::Color;

use crate::model::{Model, ServerState};

/// Stable per-server display color, assigned by positional index.
///
/// Provisional palette (contract Q4 open question); the mapping lives here
/// for easy revision.
pub fn server_color(index: usize) -> Color {
    match index {
        0 => Color::Cyan,
        1 => Color::Yellow,
        2 => Color::Magenta,
        _ => Color::White,
    }
}

/// Top-level layout descriptor.
///
/// The actual rendering is driven by `app.rs`, which calls the widget
/// functions in each sub-module.  This struct is a convenience holder for
/// parameters shared across render calls.
#[derive(Debug, Clone)]
pub struct RenderContext<'a> {
    pub model: &'a Model,
}

impl<'a> RenderContext<'a> {
    pub fn new(model: &'a Model) -> Self {
        Self { model }
    }

    pub fn servers(&self) -> &[ServerState] {
        self.model.servers()
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

/// Format an `Option<T: Display>` for display, showing "n/a" when `None`.
pub fn fmt_opt<T: std::fmt::Display>(v: Option<&T>) -> String {
    match v {
        Some(t) => t.to_string(),
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
        // Out of range falls back to White.
        assert_eq!(server_color(99), Color::White);
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
