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

/// Mirrors the provisional ingest poll cadence (src/ingest/mod.rs).
/// Both values must change together if contract Q10 resolves differently.
/// At CAPACITY=600 samples this gives a 600 s (10 min) history window.
const POLL_SECS_PER_SAMPLE: f64 = 1.0;

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

    // Compute Y bounds across all data (minimum span 1000 µs = 1 ms).
    let (y_min, y_max) = y_bounds(&server_data, 1000.0);
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
                .bounds([0.0, x_max])
                .labels(x_labels(x_max)),
        )
        .y_axis(
            Axis::default()
                .title("μs")
                .style(Style::default().fg(Color::Gray))
                .bounds([y_min, y_max])
                .labels(y_labels(y_min, y_max)),
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

    // Compute Y bounds across all data (minimum span 1.0 ppm).
    let (y_min, y_max) = y_bounds(&server_data, 1.0);
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
                .bounds([0.0, x_max])
                .labels(x_labels(x_max)),
        )
        .y_axis(
            Axis::default()
                .title("ppm")
                .style(Style::default().fg(Color::Gray))
                .bounds([y_min, y_max])
                .labels(y_labels(y_min, y_max)),
        );

    f.render_widget(chart, area);
}

/// Compute Y-axis bounds with a small margin and a minimum span.
///
/// `min_span` prevents a flat/near-flat series from producing unreadable ticks.
/// When the data range is smaller than `min_span`, the bounds are expanded
/// symmetrically about the data midpoint before the 10 % margin is applied.
/// The empty-series fallback is `(-min_span/2, min_span/2)`.
fn y_bounds(series: &[Vec<(f64, f64)>], min_span: f64) -> (f64, f64) {
    let all_y: Vec<f64> = series
        .iter()
        .flat_map(|d| d.iter().map(|(_, y)| *y))
        .collect();

    if all_y.is_empty() {
        return (-min_span / 2.0, min_span / 2.0);
    }

    let raw_min = all_y.iter().cloned().fold(f64::INFINITY, f64::min);
    let raw_max = all_y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // Enforce minimum span by expanding symmetrically around the midpoint.
    let (data_min, data_max) = if (raw_max - raw_min) < min_span {
        let mid = (raw_min + raw_max) / 2.0;
        (mid - min_span / 2.0, mid + min_span / 2.0)
    } else {
        (raw_min, raw_max)
    };

    let margin = ((data_max - data_min) * 0.1).max(0.001);
    (data_min - margin, data_max + margin)
}

/// Format a Y-axis tick value with adaptive decimal precision.
///
/// Decimals are chosen based on the axis span magnitude so labels stay compact:
/// span ≥ 100 → 0 decimals, ≥ 10 → 1, ≥ 1 → 2, else 3.
fn fmt_axis_value(v: f64, span: f64) -> String {
    let decimals = if span >= 100.0 {
        0
    } else if span >= 10.0 {
        1
    } else if span >= 1.0 {
        2
    } else {
        3
    };
    format!("{v:.decimals$}")
}

/// Format an elapsed-time X-axis label in whole seconds.
///
/// The origin (seconds == 0.0) is rendered as `"0s"`.
/// All other values are rendered as `"-{n}s"` where n is a positive integer.
fn fmt_elapsed_label(seconds: f64) -> String {
    if seconds == 0.0 {
        "0s".to_string()
    } else {
        format!("-{}s", (-seconds).round() as u64)
    }
}

/// Generate 5 evenly-spaced Y-axis tick labels (bottom → top).
///
/// ratatui distributes labels evenly across the axis, so the first label maps
/// to the lower bound and the last to the upper bound.
fn y_labels(y_min: f64, y_max: f64) -> Vec<String> {
    let span = y_max - y_min;
    (0..=4)
        .map(|k| {
            let v = y_min + span * (k as f64) / 4.0;
            fmt_axis_value(v, span)
        })
        .collect()
}

/// Generate 4 evenly-spaced X-axis elapsed-time labels (oldest → newest).
///
/// Labels represent elapsed seconds relative to the newest sample (right edge = 0s).
/// At full CAPACITY=600 samples the left edge label is approximately `-600s`.
fn x_labels(x_max: f64) -> Vec<String> {
    (0..=3)
        .map(|k| {
            // Fraction of x_max corresponding to this tick position.
            let x = x_max * (k as f64) / 3.0;
            // Elapsed seconds: newest sample is 0s; older samples are negative.
            let elapsed = (x - x_max) * POLL_SECS_PER_SAMPLE;
            fmt_elapsed_label(elapsed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn y_bounds_empty() {
        let (min, max) = y_bounds(&[], 1000.0);
        assert!(min < max);
        // Empty fallback should be at least min_span wide before margin.
        assert!((max - min) >= 1000.0);
    }

    #[test]
    fn y_bounds_single_point() {
        let data = vec![vec![(0.0, 5.0)]];
        let (min, max) = y_bounds(&data, 1000.0);
        assert!(min < 5.0);
        assert!(max > 5.0);
        // Single point should be widened to at least min_span.
        assert!((max - min) >= 1000.0);
    }

    #[test]
    fn y_bounds_range() {
        let data = vec![vec![(0.0, -10.0), (1.0, 10.0)]];
        let (min, max) = y_bounds(&data, 1.0);
        assert!(min < -10.0);
        assert!(max > 10.0);
    }

    #[test]
    fn y_bounds_flat_series_widened_to_min_span() {
        // Series with a span much smaller than min_span should be expanded.
        let data = vec![vec![(0.0, 0.1), (1.0, 0.2)]];
        let (min, max) = y_bounds(&data, 1000.0);
        assert!((max - min) >= 1000.0);
    }

    #[test]
    fn fmt_elapsed_label_origin() {
        assert_eq!(fmt_elapsed_label(0.0), "0s");
    }

    #[test]
    fn fmt_elapsed_label_negative() {
        assert_eq!(fmt_elapsed_label(-600.0), "-600s");
        assert_eq!(fmt_elapsed_label(-1.0), "-1s");
    }

    #[test]
    fn fmt_axis_value_decimals() {
        // span >= 100 → 0 decimals
        assert_eq!(fmt_axis_value(12.5, 200.0), "12");
        // span >= 10 → 1 decimal
        assert_eq!(fmt_axis_value(12.5, 50.0), "12.5");
        // span >= 1 → 2 decimals
        assert_eq!(fmt_axis_value(1.5, 5.0), "1.50");
        // span < 1 → 3 decimals
        assert_eq!(fmt_axis_value(0.1234, 0.5), "0.123");
    }

    #[test]
    fn y_labels_count() {
        let labels = y_labels(-500.0, 500.0);
        assert_eq!(labels.len(), 5);
    }

    #[test]
    fn x_labels_count() {
        let labels = x_labels(600.0);
        assert_eq!(labels.len(), 4);
        // Last label (newest, right edge) should always be 0s.
        assert_eq!(labels.last().unwrap(), "0s");
    }
}
