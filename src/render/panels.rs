//! Per-server detail panels.
//!
//! Reads model snapshots only.  No terminal I/O, no mutation.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use super::{fmt_opt_f64, format_status, server_color};
use crate::model::ServerState;

/// Render a detail panel for one server, filling `area`.
pub fn render_server_panel(f: &mut Frame, area: Rect, server: &ServerState) {
    let color = server_color(server.index);
    let title = format!(" {} [{}] ", server.hostname, format_status(server));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split inner area: top = NTP, bottom = GPS.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    render_ntp_detail(f, chunks[0], server);
    render_gps_detail(f, chunks[1], server);
}

fn render_ntp_detail(f: &mut Frame, area: Rect, server: &ServerState) {
    let ntp = server.latest_ntp();

    let items: Vec<ListItem> = if let Some(n) = ntp {
        vec![
            ListItem::new(Line::from(vec![
                Span::styled("Reference: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(n.reference.as_deref().unwrap_or("n/a")),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("Stratum:   ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(
                    n.stratum
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "n/a".to_owned()),
                ),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("Offset:    ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(
                    "{} μs",
                    fmt_opt_f64(n.offset_sec.map(|v| v * 1e6), 3)
                )),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("RMS Off:   ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(
                    "{} μs",
                    fmt_opt_f64(n.rms_offset_sec.map(|v| v * 1e6), 3)
                )),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("Freq:      ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("{} ppm", fmt_opt_f64(n.freq_ppm, 3))),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("Root del:  ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(
                    "{} ms",
                    fmt_opt_f64(n.root_delay_sec.map(|v| v * 1e3), 3)
                )),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("Root dis:  ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(
                    "{} ms",
                    fmt_opt_f64(n.root_dispersion_sec.map(|v| v * 1e3), 3)
                )),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("Leap:      ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(n.leap_status.as_deref().unwrap_or("n/a")),
            ])),
        ]
    } else {
        vec![ListItem::new("NTP: no data")]
    };

    let list = List::new(items).block(Block::default().title("NTP/Chrony").borders(Borders::TOP));
    f.render_widget(list, area);
}

fn render_gps_detail(f: &mut Frame, area: Rect, server: &ServerState) {
    let gps = server.latest_gps();

    let text: Vec<Line> = if let Some(g) = gps {
        let fix_str = match g.fix_mode {
            Some(0) | None => "unknown",
            Some(1) => "no fix",
            Some(2) => "2D fix",
            Some(3) => "3D fix",
            Some(_) => "fix",
        };
        let used: usize = g.satellites.iter().filter(|s| s.used).count();
        let total: usize = g.satellites.len();

        vec![
            Line::from(format!("Fix:  {fix_str}")),
            Line::from(format!("Lat:  {}", fmt_opt_f64(g.lat, 6))),
            Line::from(format!("Lon:  {}", fmt_opt_f64(g.lon, 6))),
            Line::from(format!("Alt:  {} m", fmt_opt_f64(g.alt_m, 1))),
            Line::from(format!("Sats: {used}/{total} used")),
        ]
    } else {
        vec![Line::from("GPS: no data")]
    };

    let para = Paragraph::new(text).block(Block::default().title("GPS").borders(Borders::TOP));
    f.render_widget(para, area);
}
