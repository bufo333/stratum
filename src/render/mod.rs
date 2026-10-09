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

/// Selected content of the middle row; orchestration owns this state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MiddleView {
    #[default]
    Graphs,
    Timing,
}

/// Required dimensions refer only to the middle rectangle.
fn timing_geometry(count: usize, width: u16) -> (u16, u16, u16) {
    let count = count.max(1) as u16;
    let columns = count.min(3).min((width / 38).max(1));
    (columns, columns * 38, count.div_ceil(columns) * 5)
}

fn dashboard_rows(area: Rect) -> [Rect; 3] {
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let chunks = Layout::vertical([
        Constraint::Percentage(20),
        Constraint::Percentage(40),
        Constraint::Percentage(40),
    ])
    .split(body);
    [chunks[0], chunks[1], chunks[2]]
}

pub fn draw(f: &mut ratatui::Frame, model: &Model, view: MiddleView, now: Instant) {
    let area = f.area();
    let [summary, middle, details] = dashboard_rows(area);
    let (columns, required_width, required_height) =
        timing_geometry(model.server_count(), middle.width);
    let fits = middle.width >= required_width && middle.height >= required_height;
    table::render_summary_table(f, summary, model);
    render_details(f, details, model);
    if view == MiddleView::Timing && fits {
        clocks::render(f, middle, model, columns, now);
    } else {
        render_graphs(f, middle, model);
    }
    if area.height > 0 {
        let footer = match view {
            MiddleView::Graphs => "Graphs | Tab: timing | q: quit".to_owned(),
            MiddleView::Timing if fits => "Timing | Tab: graphs | q: quit".to_owned(),
            MiddleView::Timing => format!(
                "Timing needs middle {required_width}x{required_height} | Tab: graphs | q: quit"
            ),
        };
        f.render_widget(
            Paragraph::new(footer),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
}

fn render_graphs(f: &mut ratatui::Frame, graphs: Rect, model: &Model) {
    // Graphs: offset | freq | sky plot.
    let graph_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(graphs);

    graphs::render_offset_graph(f, graph_chunks[0], model);
    graphs::render_freq_graph(f, graph_chunks[1], model);
    skyplot::render_sky_plot(f, graph_chunks[2], model);
}

fn render_details(f: &mut ratatui::Frame, details: Rect, model: &Model) {
    // Server panels: one per server, evenly distributed.
    let n = model.server_count().max(1);
    let panel_constraints: Vec<Constraint> =
        (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(panel_constraints)
        .split(details);

    for (i, server) in model.servers().iter().enumerate() {
        if let Some(chunk) = panel_chunks.get(i) {
            panels::render_server_panel(f, *chunk, server);
        }
    }
}
