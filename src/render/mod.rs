//! Widget composition root.
//!
//! Reads model snapshots only — no terminal I/O, no crossterm events, no
//! network or file I/O, no model mutation.

pub mod clocks;
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

use crate::model::Model;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::Paragraph,
};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Timing,
    Dashboard,
}
impl View {
    pub fn toggled(self) -> Self {
        match self {
            Self::Timing => Self::Dashboard,
            Self::Dashboard => Self::Timing,
        }
    }
}

pub fn draw(f: &mut ratatui::Frame, model: &Model, view: View, now: Instant) {
    let area = f.area();
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    match view {
        View::Timing => clocks::render(f, body, model, now),
        View::Dashboard => dashboard(f, body, model),
    }
    if area.height > 0 {
        f.render_widget(
            Paragraph::new("Tab: dashboard / timing   q: quit"),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
}

/// Build and render the TUI layout from a model snapshot.
///
/// Layout (top → bottom):
///   - Summary table (20% height)
///   - Graphs area (40% height): offset (~34%) | freq (~33%) | sky plot (~33%)
///   - Server panels (40% height): one panel per server
fn dashboard(f: &mut ratatui::Frame, size: Rect, model: &Model) {
    // Top-level vertical split.
    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(size);

    // Summary table.
    table::render_summary_table(f, top_chunks[0], model);

    // Graphs: offset | freq | sky plot.
    let graph_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(top_chunks[1]);

    graphs::render_offset_graph(f, graph_chunks[0], model);
    graphs::render_freq_graph(f, graph_chunks[1], model);
    skyplot::render_sky_plot(f, graph_chunks[2], model);

    // Server panels: one per server, evenly distributed.
    let n = model.server_count().max(1);
    let panel_constraints: Vec<Constraint> =
        (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(panel_constraints)
        .split(top_chunks[2]);

    for (i, server) in model.servers().iter().enumerate() {
        if let Some(chunk) = panel_chunks.get(i) {
            panels::render_server_panel(f, *chunk, server);
        }
    }
}
