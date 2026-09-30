//! Summary table widget.
//!
//! Reads model snapshots only.  No terminal I/O, no mutation.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Frame,
};

use super::{fmt_opt_f64, format_status, server_color};
use crate::model::Model;

/// Render a summary table with one row per server.
///
/// Columns: Host | Status | Stratum | Offset (μs) | Freq (ppm) | Fix | Sats
pub fn render_summary_table(f: &mut Frame, area: Rect, model: &Model) {
    let header = Row::new(vec![
        Cell::from("Host").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Stratum").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Offset (μs)").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Freq (ppm)").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("GPS Fix").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Sats").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = model
        .servers()
        .iter()
        .map(|s| {
            let color = server_color(s.index);
            let ntp = s.latest_ntp();
            let gps = s.latest_gps();

            let stratum = ntp
                .and_then(|n| n.stratum)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_owned());

            let offset = fmt_opt_f64(ntp.and_then(|n| n.offset_sec).map(|v| v * 1e6), 3);
            let freq = fmt_opt_f64(ntp.and_then(|n| n.freq_ppm), 3);

            let fix_str = match gps.and_then(|g| g.fix_mode) {
                None => "n/a",
                Some(0) => "unknown",
                Some(1) => "no fix",
                Some(2) => "2D",
                Some(3) => "3D",
                Some(_) => "fix",
            };

            let sats = match gps {
                Some(g) => {
                    let used = g.satellites.iter().filter(|s| s.used).count();
                    format!("{}/{}", used, g.satellites.len())
                }
                None => "n/a".to_owned(),
            };

            Row::new(vec![
                Cell::from(s.hostname.clone()).style(Style::default().fg(color)),
                Cell::from(format_status(s)),
                Cell::from(stratum),
                Cell::from(offset),
                Cell::from(freq),
                Cell::from(fix_str),
                Cell::from(sats),
            ])
            .style(Style::default().fg(color))
        })
        .collect();

    let widths = [
        ratatui::layout::Constraint::Percentage(20),
        ratatui::layout::Constraint::Percentage(12),
        ratatui::layout::Constraint::Percentage(10),
        ratatui::layout::Constraint::Percentage(15),
        ratatui::layout::Constraint::Percentage(15),
        ratatui::layout::Constraint::Percentage(10),
        ratatui::layout::Constraint::Percentage(10),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .title("Server Summary")
            .borders(Borders::ALL),
    );

    f.render_stateful_widget(table, area, &mut TableState::default());
}
