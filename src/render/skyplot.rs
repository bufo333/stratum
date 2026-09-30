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

/// Canvas-space x offset applied to a satellite label relative to its dot,
/// so overlapping dots still show distinct labels (~one cell right).
const LABEL_OFFSET_X: f64 = 0.08;
/// Canvas-space y offset applied to a satellite label relative to its dot
/// (~half a cell up).
const LABEL_OFFSET_Y: f64 = 0.06;
/// Minimum canvas-space distance two satellite dots must keep; closer pairs are
/// nudged apart (~two x-cells / one y-cell).
const MIN_DIST: f64 = 0.12;
/// Canvas-space x step applied per nudge iteration when separating dots.
const NUDGE_STEP: f64 = 0.08;
/// Maximum nudge iterations per satellite before giving up (bounds the loop).
const MAX_NUDGES: usize = 12;
/// Guard against division by zero when two satellite dots are exactly coincident.
const NUDGE_EPSILON: f64 = 1e-9;

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

/// Build a dot-only `Line` for a satellite entry in the sky plot.
///
/// Each entry in `servers` is `(server_index, used)`, pre-sorted ascending by
/// server index.  For each entry one dot span is pushed — filled `●` when
/// `used == true`, hollow `○` otherwise — coloured with that server's palette
/// colour.  No label span is included; the label is printed separately via
/// `build_label_line`.
///
/// The returned `Line` is `'static`: glyph literals are `&'static str`.
pub fn build_dot_line(servers: &[(usize, bool)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(servers.len());
    for &(idx, used) in servers {
        let glyph: &'static str = if used { "●" } else { "○" };
        spans.push(Span::styled(glyph, Style::default().fg(server_color(idx))));
    }
    Line::from(spans)
}

/// Build a label-only `Line` for a satellite entry in the sky plot.
///
/// Returns a single white span containing the satellite label (e.g. `"G01"`,
/// `"R05"`, `"23"`).  No leading space — the caller positions the label
/// geometrically via `LABEL_OFFSET_X`/`LABEL_OFFSET_Y`.
///
/// The returned `Line` is `'static` (owns its `String`).
pub fn build_label_line(prn: i32, gnss_id: Option<&str>) -> Line<'static> {
    Line::from(Span::styled(
        sat_label(prn, gnss_id),
        Style::default().fg(Color::White),
    ))
}

/// Clamp `(x, y)` to the horizon circle.
///
/// Because x is scaled by `ASPECT`, the horizon is the unit circle in
/// `(x/ASPECT, y)`.  Points already inside are returned unchanged; points
/// outside are scaled back onto the ring.
fn clamp_to_horizon(x: f64, y: f64) -> (f64, f64) {
    let nx = x / ASPECT;
    let r = (nx * nx + y * y).sqrt();
    if r > 1.0 {
        ((nx / r) * ASPECT, y / r)
    } else {
        (x, y)
    }
}

/// Resolve dot collisions deterministically.
///
/// `positions` are dot centres in canvas coordinates, in a stable order (the
/// caller passes them in `BTreeMap` key order).  Returns the same number of
/// positions; the first is never moved.  Each later position that lands within
/// `MIN_DIST` of an already-placed one is nudged `NUDGE_STEP` along the unit
/// vector pointing away from the nearest colliding neighbor (up to `MAX_NUDGES`
/// times) until it clears every placed point or the step budget is exhausted,
/// then clamped to the horizon.  When the nearest neighbor is closer than
/// `NUDGE_EPSILON` (exactly coincident), the direction is undefined and a fixed
/// `+x` step is applied instead.
fn spread_positions(positions: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut placed: Vec<(f64, f64)> = Vec::with_capacity(positions.len());
    for &(px, py) in positions {
        let (mut x, mut y) = (px, py);
        for _ in 0..MAX_NUDGES {
            // find nearest already-placed colliding neighbor
            let nearest = placed
                .iter()
                .filter(|&&(qx, qy)| {
                    let dx = x - qx;
                    let dy = y - qy;
                    (dx * dx + dy * dy).sqrt() < MIN_DIST
                })
                .min_by(|&&(ax, ay), &&(bx, by)| {
                    let da = (x - ax) * (x - ax) + (y - ay) * (y - ay);
                    let db = (x - bx) * (x - bx) + (y - by) * (y - by);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                });
            if let Some(&(qx, qy)) = nearest {
                let dx = x - qx;
                let dy = y - qy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < NUDGE_EPSILON {
                    x += NUDGE_STEP; // coincident fallback
                } else {
                    x += (dx / dist) * NUDGE_STEP;
                    y += (dy / dist) * NUDGE_STEP;
                }
            } else {
                break;
            }
        }
        let (x, y) = clamp_to_horizon(x, y);
        placed.push((x, y));
    }
    placed
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

            // Build drawables in BTreeMap key order (deterministic).
            let drawables: Vec<(f64, f64, Line<'static>, Line<'static>)> = sat_map
                .iter()
                .map(|(&(prn, _prefix), (pos, gnss_id, entries))| {
                    let (px, py) = *pos;
                    let mut sorted_entries = entries.clone();
                    sorted_entries.sort_by_key(|&(idx, _)| idx);
                    let dot = build_dot_line(&sorted_entries);
                    let label = build_label_line(prn, gnss_id.as_deref());
                    (px, py, dot, label)
                })
                .collect();

            // Nudge coincident dot positions apart, keeping dots on-horizon.
            let raw_positions: Vec<(f64, f64)> =
                drawables.iter().map(|&(x, y, _, _)| (x, y)).collect();
            let nudged = spread_positions(&raw_positions);

            for ((_, _, dot, label), (nx, ny)) in drawables.into_iter().zip(nudged) {
                ctx.print(nx, ny, dot);
                ctx.print(nx + LABEL_OFFSET_X, ny + LABEL_OFFSET_Y, label);
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

    // build_dot_line

    #[test]
    fn dot_line_three_servers_mixed() {
        // servers 0 (used), 1 (used), 2 (not used) → 3 dot spans, no label
        let line = build_dot_line(&[(0, true), (1, true), (2, false)]);
        assert_eq!(line.spans.len(), 3, "expected 3 dot spans, no label");
        assert_eq!(line.spans[0].content, "●");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].content, "●");
        assert_eq!(line.spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(line.spans[2].content, "○");
        assert_eq!(line.spans[2].style.fg, Some(Color::Magenta));
    }

    #[test]
    fn dot_line_single_server() {
        // server 0 not used → 1 hollow dot span
        let line = build_dot_line(&[(0, false)]);
        assert_eq!(line.spans.len(), 1, "expected 1 dot span");
        assert_eq!(line.spans[0].content, "○");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn dot_line_subset_no_yellow() {
        // servers 0 and 2 only — no Yellow dot for server 1
        let line = build_dot_line(&[(0, true), (2, true)]);
        assert_eq!(line.spans.len(), 2, "expected 2 dot spans");
        assert_eq!(line.spans[0].content, "●");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].content, "●");
        assert_eq!(line.spans[1].style.fg, Some(Color::Magenta));
    }

    // build_label_line

    #[test]
    fn label_line_gps_zero_padded() {
        let line = build_label_line(1, Some("GPS"));
        assert_eq!(line.spans.len(), 1, "expected single label span");
        assert_eq!(line.spans[0].content, "G01");
        assert_eq!(line.spans[0].style.fg, Some(Color::White));
    }

    #[test]
    fn label_line_bare_prn() {
        let line = build_label_line(23, None);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "23");
        assert_eq!(line.spans[0].style.fg, Some(Color::White));
    }

    #[test]
    fn label_line_large_prn() {
        // Guards against truncation of PRNs > 99.
        let line = build_label_line(123, Some("GPS"));
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "G123");
        assert_eq!(line.spans[0].style.fg, Some(Color::White));
    }

    // spread_positions

    #[test]
    fn spread_leaves_distant_points() {
        let pts = vec![(-0.5, 0.0), (0.5, 0.0)];
        let out = spread_positions(&pts);
        assert!(
            (out[0].0 - pts[0].0).abs() < EPS,
            "first moved unexpectedly"
        );
        assert!(
            (out[1].0 - pts[1].0).abs() < EPS,
            "second moved unexpectedly"
        );
    }

    #[test]
    fn spread_first_never_moves() {
        let pts = vec![(0.1, 0.1), (0.1, 0.1), (0.1, 0.1)];
        let out = spread_positions(&pts);
        assert!(
            (out[0].0 - pts[0].0).abs() < EPS && (out[0].1 - pts[0].1).abs() < EPS,
            "first element must not move"
        );
    }

    #[test]
    fn spread_separates_coincident() {
        let pts = vec![(0.1, 0.1), (0.1, 0.1)];
        let out = spread_positions(&pts);
        // First element is unchanged.
        assert!((out[0].0 - 0.1).abs() < EPS && (out[0].1 - 0.1).abs() < EPS);
        // Second is at least MIN_DIST away from first.
        let dist = ((out[1].0 - out[0].0).powi(2) + (out[1].1 - out[0].1).powi(2)).sqrt();
        assert!(dist >= MIN_DIST - EPS, "dist={dist} < MIN_DIST={MIN_DIST}");
    }

    #[test]
    fn spread_is_deterministic() {
        let pts = vec![(0.0, 0.0), (0.0, 0.0), (0.05, 0.05)];
        let out1 = spread_positions(&pts);
        let out2 = spread_positions(&pts);
        for (a, b) in out1.iter().zip(out2.iter()) {
            assert!(
                (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS,
                "non-deterministic: {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn spread_separates_vertically_coincident() {
        // Two points close on the y-axis — x-only nudge would fail to separate them
        let positions = vec![(0.0_f64, 0.0_f64), (0.0_f64, 0.1_f64)];
        let result = spread_positions(&positions);
        assert_eq!(result[0], positions[0]); // first never moves
        let (x0, y0) = result[0];
        let (x1, y1) = result[1];
        let dist = ((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)).sqrt();
        let eps = 1e-9_f64;
        assert!(
            dist >= MIN_DIST - eps,
            "vertically close satellites not separated: dist={dist}"
        );
    }

    #[test]
    fn spread_clamps_to_horizon() {
        // A cluster near the horizon east that will be nudged rightward.
        let pts = vec![(1.8, 0.0), (1.8, 0.0), (1.8, 0.0)];
        let out = spread_positions(&pts);
        for &(x, y) in &out {
            let nx = x / ASPECT;
            let r_sq = nx * nx + y * y;
            assert!(
                r_sq <= 1.0 + EPS,
                "point outside horizon: ({x},{y}), r²={r_sq}"
            );
        }
    }

    // clamp_to_horizon

    #[test]
    fn clamp_inside_unchanged() {
        let (x, y) = clamp_to_horizon(0.0, 0.5);
        assert!((x - 0.0).abs() < EPS);
        assert!((y - 0.5).abs() < EPS);
    }

    #[test]
    fn clamp_outside_pulled_to_ring() {
        // (3.0, 0.0) → normalized radius = 3.0/ASPECT = 1.5, which is >1.
        let (x, y) = clamp_to_horizon(3.0, 0.0);
        let nx = x / ASPECT;
        let r = (nx * nx + y * y).sqrt();
        assert!((r - 1.0).abs() < EPS, "r={r}");
    }
}
