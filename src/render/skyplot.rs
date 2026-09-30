//! Polar sky-plot widget — combined view of all above-horizon satellites from
//! all connected servers, colour-coded by server index.
//!
//! Reads model snapshots only — no terminal I/O, no model mutation, no ingest
//! imports.

use std::collections::BTreeMap;

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Points},
        Block, Borders,
    },
    Frame,
};

use crate::{model::Model, render::server_color};

/// Horizontal scale factor applied to all x coordinates.
///
/// Terminal cells are roughly twice as tall as wide, so a mathematically
/// circular plot renders as a vertical ellipse without correction.  This
/// single-point constant approximates a round appearance; it is applied
/// identically to rings and satellite positions so placement stays
/// geometrically consistent.
const ASPECT: f64 = 2.0;

/// Angular step (degrees) used when sampling parametric rings.
const RING_STEP_DEG: f64 = 3.0;

/// Convert polar sky coordinates to canvas (x, y).
///
/// Elevation is clamped to `[0.0, 90.0]`.  Azimuth is normalised into
/// `[0.0, 360.0)`.
///
/// Coordinate conventions:
/// - Zenith (elevation 90°) maps to (0, 0).
/// - Horizon (elevation 0°) has radius 1.0.
/// - North (azimuth 0°) is at the top (positive y).
/// - East  (azimuth 90°) is at the right (positive x).
pub fn polar_to_xy(elevation: f64, azimuth: f64) -> (f64, f64) {
    let el = elevation.clamp(0.0, 90.0);
    let az = azimuth.rem_euclid(360.0);
    let r = 1.0 - (el / 90.0);
    let rad = az.to_radians();
    let x = r * rad.sin() * ASPECT;
    let y = r * rad.cos();
    (x, y)
}

/// Return the single-letter constellation prefix for a known gpsd constellation
/// name, or `None` for unknown/`None` input.
///
/// Matching is case-insensitive.  Only recognised names produce a letter; no
/// letter is invented for an unrecognised value.
pub fn constellation_prefix(gnss_id: Option<&str>) -> Option<char> {
    let id = gnss_id?;
    match id.to_lowercase().as_str() {
        "gps" => Some('G'),
        "glonass" => Some('R'),
        "galileo" => Some('E'),
        "beidou" => Some('C'),
        "qzss" => Some('J'),
        "sbas" => Some('S'),
        "irnss" | "navic" => Some('I'),
        _ => None,
    }
}

/// Build the satellite label string.
///
/// When a constellation prefix is available the result is `<prefix><prn>`
/// zero-padded to at least two digits, e.g. `G01` or `R05`.  When no prefix
/// is available the bare PRN is used, e.g. `23`.
pub fn sat_label(prn: i32, gnss_id: Option<&str>) -> String {
    match constellation_prefix(gnss_id) {
        Some(prefix) => format!("{}{:02}", prefix, prn),
        None => format!("{}", prn),
    }
}

/// Build a multi-span `Line` for a satellite entry in the sky plot.
///
/// Each entry in `servers` is `(server_index, used)`, pre-sorted ascending by
/// server index.  For each entry one dot span is pushed — filled `●` when
/// `used == true`, hollow `○` otherwise — coloured with that server's palette
/// colour.  A single trailing label span is appended, e.g. `" G01"`.
///
/// The returned `Line` is `'static`: glyph literals are `&'static str` and the
/// label is an owned `String`.
pub fn build_sat_line(servers: &[(usize, bool)], prn: i32, gnss_id: Option<&str>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(servers.len() + 1);
    for &(idx, used) in servers {
        let glyph: &'static str = if used { "●" } else { "○" };
        spans.push(Span::styled(glyph, Style::default().fg(server_color(idx))));
    }
    spans.push(Span::styled(
        format!(" {}", sat_label(prn, gnss_id)),
        Style::default().fg(Color::White),
    ));
    Line::from(spans)
}

/// Sample a ring of a given sky-plot radius as a set of canvas (x, y) points.
fn ring_points(radius: f64) -> Vec<(f64, f64)> {
    let steps = (360.0 / RING_STEP_DEG).ceil() as usize + 1;
    (0..steps)
        .map(|i| {
            let az = (i as f64) * RING_STEP_DEG;
            let rad = az.to_radians();
            let x = radius * rad.sin() * ASPECT;
            let y = radius * rad.cos();
            (x, y)
        })
        .collect()
}

/// Map key: `(prn, constellation_prefix)`. Value: canvas position, optional
/// `gnss_id` string, and the list of `(server_index, used)` entries.
type SatMap = BTreeMap<(i32, Option<char>), ((f64, f64), Option<String>, Vec<(usize, bool)>)>;

/// Render the combined polar sky-plot into `area`.
///
/// Draws horizon / 30° / 60° rings, cardinal labels, and one labelled dot per
/// above-horizon satellite from every server.  When no plottable satellites
/// exist the grid is still drawn with a `no satellites` hint.
pub fn render_sky_plot(f: &mut Frame, area: Rect, model: &Model) {
    // Group plottable satellites by (prn, constellation_prefix) across all servers.
    // Value: ((canvas_x, canvas_y), representative_gnss_id, [(server_index, used)])
    let mut sat_map: SatMap = BTreeMap::new();
    for server in model.servers() {
        if let Some(gps) = server.latest_gps() {
            for sat in &gps.satellites {
                let (Some(prn), Some(el), Some(az)) = (sat.prn, sat.elevation, sat.azimuth) else {
                    continue;
                };
                let used = sat.used == Some(true);
                let key = (prn, constellation_prefix(sat.gnss_id.as_deref()));
                sat_map
                    .entry(key)
                    .and_modify(|(_, _, entries)| entries.push((server.index, used)))
                    .or_insert_with(|| {
                        let pos = polar_to_xy(el, az);
                        (pos, sat.gnss_id.clone(), vec![(server.index, used)])
                    });
            }
        }
    }
    let have_sats = !sat_map.is_empty();

    let block = Block::default()
        .title("Sky (all servers)")
        .borders(Borders::ALL);

    let canvas = Canvas::default()
        .block(block)
        .x_bounds([-1.2, 1.2])
        .y_bounds([-1.2, 1.2])
        .paint(move |ctx| {
            // --- Layer 0: rings and cardinal labels ---
            let dim = Color::DarkGray;

            // Horizon (elevation 0°), 30°, and 60° rings.
            for radius in [1.0_f64, 1.0 - 30.0 / 90.0, 1.0 - 60.0 / 90.0] {
                let pts = ring_points(radius);
                ctx.draw(&Points {
                    coords: &pts,
                    color: dim,
                });
            }

            // Cardinal labels just outside the horizon ring.
            let label_r = 1.08_f64;
            ctx.print(
                0.0,
                label_r,
                Line::styled("N", Style::default().fg(Color::Gray)),
            );
            ctx.print(
                label_r * ASPECT,
                0.0,
                Line::styled("E", Style::default().fg(Color::Gray)),
            );
            ctx.print(
                0.0,
                -label_r,
                Line::styled("S", Style::default().fg(Color::Gray)),
            );
            ctx.print(
                -label_r * ASPECT,
                0.0,
                Line::styled("W", Style::default().fg(Color::Gray)),
            );

            // Empty-state hint.
            if !have_sats {
                ctx.print(
                    -0.55,
                    0.0,
                    Line::styled("no satellites", Style::default().fg(Color::DarkGray)),
                );
            }

            // --- Layer 1: satellites ---
            ctx.layer();

            for (&(prn, _prefix), (pos, gnss_id, entries)) in &sat_map {
                let (x, y) = *pos;
                let mut sorted_entries = entries.clone();
                sorted_entries.sort_by_key(|&(idx, _)| idx);
                ctx.print(
                    x,
                    y,
                    build_sat_line(&sorted_entries, prn, gnss_id.as_deref()),
                );
            }
        });

    f.render_widget(canvas, area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    // polar_to_xy

    #[test]
    fn zenith_maps_to_origin() {
        let (x, y) = polar_to_xy(90.0, 0.0);
        assert!(x.abs() < EPS, "x={x}");
        assert!(y.abs() < EPS, "y={y}");
    }

    #[test]
    fn horizon_north_maps_to_top() {
        let (x, y) = polar_to_xy(0.0, 0.0);
        assert!(x.abs() < EPS, "x={x}");
        assert!((y - 1.0).abs() < EPS, "y={y}");
    }

    #[test]
    fn horizon_east_has_positive_x_and_zero_y() {
        let (x, y) = polar_to_xy(0.0, 90.0);
        assert!(x > 0.0, "x={x}");
        assert!(y.abs() < EPS, "y={y}");
    }

    #[test]
    fn horizon_south_maps_to_bottom() {
        let (x, y) = polar_to_xy(0.0, 180.0);
        assert!(x.abs() < EPS, "x={x}");
        assert!((y + 1.0).abs() < EPS, "y={y}");
    }

    #[test]
    fn horizon_west_has_negative_x_and_zero_y() {
        let (x, y) = polar_to_xy(0.0, 270.0);
        assert!(x < 0.0, "x={x}");
        assert!(y.abs() < EPS, "y={y}");
    }

    #[test]
    fn aspect_applied_to_east_x() {
        // At horizon East the expected x = 1.0 * ASPECT, y ≈ 0.
        let (x, _) = polar_to_xy(0.0, 90.0);
        assert!((x - ASPECT).abs() < EPS, "x={x}, ASPECT={ASPECT}");
    }

    // constellation_prefix

    #[test]
    fn gps_prefix() {
        assert_eq!(constellation_prefix(Some("GPS")), Some('G'));
    }

    #[test]
    fn glonass_case_insensitive() {
        assert_eq!(constellation_prefix(Some("glonass")), Some('R'));
    }

    #[test]
    fn galileo_prefix() {
        assert_eq!(constellation_prefix(Some("Galileo")), Some('E'));
    }

    #[test]
    fn beidou_prefix() {
        assert_eq!(constellation_prefix(Some("BeiDou")), Some('C'));
    }

    #[test]
    fn qzss_prefix() {
        assert_eq!(constellation_prefix(Some("QZSS")), Some('J'));
    }

    #[test]
    fn sbas_prefix() {
        assert_eq!(constellation_prefix(Some("SBAS")), Some('S'));
    }

    #[test]
    fn irnss_prefix() {
        assert_eq!(constellation_prefix(Some("IRNSS")), Some('I'));
    }

    #[test]
    fn navic_prefix() {
        assert_eq!(constellation_prefix(Some("NavIC")), Some('I'));
    }

    #[test]
    fn none_gnss_id_returns_none() {
        assert_eq!(constellation_prefix(None), None);
    }

    #[test]
    fn unknown_gnss_id_returns_none() {
        assert_eq!(constellation_prefix(Some("unknown")), None);
    }

    // sat_label

    #[test]
    fn gps_label_zero_padded() {
        assert_eq!(sat_label(1, Some("GPS")), "G01");
    }

    #[test]
    fn glonass_label_zero_padded() {
        assert_eq!(sat_label(5, Some("GLONASS")), "R05");
    }

    #[test]
    fn no_gnss_id_bare_prn() {
        assert_eq!(sat_label(23, None), "23");
    }

    #[test]
    fn large_prn_not_truncated() {
        // PRNs > 99 must not be clipped.
        assert_eq!(sat_label(123, Some("GPS")), "G123");
    }

    // build_sat_line

    #[test]
    fn three_servers_mixed_used() {
        // servers 0 (used), 1 (used), 2 (not used) → 4 spans: ●●○ + label
        let line = build_sat_line(&[(0, true), (1, true), (2, false)], 1, Some("GPS"));
        assert_eq!(line.spans.len(), 4, "expected 3 dots + 1 label");
        assert_eq!(line.spans[0].content, "●");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].content, "●");
        assert_eq!(line.spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(line.spans[2].content, "○");
        assert_eq!(line.spans[2].style.fg, Some(Color::Magenta));
        assert_eq!(line.spans[3].content, " G01");
        assert_eq!(line.spans[3].style.fg, Some(Color::White));
    }

    #[test]
    fn single_server_not_used_no_constellation() {
        // server 0 not used, no gnss_id → ○ + bare PRN label
        let line = build_sat_line(&[(0, false)], 23, None);
        assert_eq!(line.spans.len(), 2, "expected 1 dot + 1 label");
        assert_eq!(line.spans[0].content, "○");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].content, " 23");
        assert_eq!(line.spans[1].style.fg, Some(Color::White));
    }

    #[test]
    fn subset_servers_no_yellow_dot() {
        // servers 0 and 2 only — no Yellow dot for server 1
        let line = build_sat_line(&[(0, true), (2, true)], 5, Some("GLONASS"));
        assert_eq!(line.spans.len(), 3, "expected 2 dots + 1 label");
        assert_eq!(line.spans[0].content, "●");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].content, "●");
        assert_eq!(line.spans[1].style.fg, Some(Color::Magenta));
        assert_eq!(line.spans[2].content, " R05");
        assert_eq!(line.spans[2].style.fg, Some(Color::White));
    }
}
