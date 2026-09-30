//! Combined multi-server color-coded timing graphs.
//!
//! Reads model history ring-buffers only.  No terminal I/O, no mutation.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
    Frame,
};

use super::server_color;
use crate::model::Model;

/// Render the combined NTP offset graph for all servers.
///
/// Each server gets one colored line representing its offset history
/// (oldest → newest, left → right).
pub fn render_offset_graph(f: &mut Frame, area: Rect, model: &Model) {
    let server_data: Vec<Vec<(f64, f64)>> = model
        .servers()
        .iter()
        .map(|s| {
            s.ntp_history()
                .iter()
                .enumerate()
                .filter_map(|(i, sample)| {
                    sample.offset_sec.map(|off| (i as f64, off * 1_000_000.0)) // μs
                })
                .collect()
        })
        .collect();

    let datasets: Vec<Dataset> = server_data
        .iter()
        .enumerate()
        .map(|(idx, data)| {
            Dataset::default()
                .name(
                    model
                        .server(idx)
                        .map(|s| s.hostname.as_str())
                        .unwrap_or("?"),
                )
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(server_color(idx)))
                .data(data)
        })
        .collect();

    // Compute Y bounds across all data.
    let (y_min, y_max) = y_bounds(&server_data);
    let x_max = server_data.iter().map(|d| d.len()).max().unwrap_or(1) as f64;

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title("NTP Offset (μs)")
                .borders(Borders::ALL),
        )
        .x_axis(
            Axis::default()
                .title("Samples")
                .style(Style::default().fg(Color::Gray))
                .bounds([0.0, x_max]),
        )
        .y_axis(
            Axis::default()
                .title("μs")
                .style(Style::default().fg(Color::Gray))
                .bounds([y_min, y_max]),
        );

    f.render_widget(chart, area);
}

/// Render the combined NTP frequency graph for all servers.
pub fn render_freq_graph(f: &mut Frame, area: Rect, model: &Model) {
    let server_data: Vec<Vec<(f64, f64)>> = model
        .servers()
        .iter()
        .map(|s| {
            s.ntp_history()
                .iter()
                .enumerate()
                .filter_map(|(i, sample)| sample.freq_ppm.map(|f| (i as f64, f)))
                .collect()
        })
        .collect();

    let datasets: Vec<Dataset> = server_data
        .iter()
        .enumerate()
        .map(|(idx, data)| {
            Dataset::default()
                .name(
                    model
                        .server(idx)
                        .map(|s| s.hostname.as_str())
                        .unwrap_or("?"),
                )
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(server_color(idx)))
                .data(data)
        })
        .collect();

    let (y_min, y_max) = y_bounds(&server_data);
    let x_max = server_data.iter().map(|d| d.len()).max().unwrap_or(1) as f64;

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title("NTP Frequency (ppm)")
                .borders(Borders::ALL),
        )
        .x_axis(
            Axis::default()
                .title("Samples")
                .style(Style::default().fg(Color::Gray))
                .bounds([0.0, x_max]),
        )
        .y_axis(
            Axis::default()
                .title("ppm")
                .style(Style::default().fg(Color::Gray))
                .bounds([y_min, y_max]),
        );

    f.render_widget(chart, area);
}

/// Compute Y-axis bounds with a small margin.
fn y_bounds(series: &[Vec<(f64, f64)>]) -> (f64, f64) {
    let all_y: Vec<f64> = series
        .iter()
        .flat_map(|d| d.iter().map(|(_, y)| *y))
        .collect();

    if all_y.is_empty() {
        return (-1.0, 1.0);
    }

    let min = all_y.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = all_y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let margin = ((max - min) * 0.1).max(0.001);
    (min - margin, max + margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn y_bounds_empty() {
        let (min, max) = y_bounds(&[]);
        assert!(min < max);
    }

    #[test]
    fn y_bounds_single_point() {
        let data = vec![vec![(0.0, 5.0)]];
        let (min, max) = y_bounds(&data);
        assert!(min < 5.0);
        assert!(max > 5.0);
    }

    #[test]
    fn y_bounds_range() {
        let data = vec![vec![(0.0, -10.0), (1.0, 10.0)]];
        let (min, max) = y_bounds(&data);
        assert!(min < -10.0);
        assert!(max > 10.0);
    }
}
