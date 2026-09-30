//! Polar sky-plot widget — combined view of all above-horizon satellites from
//! all connected servers, colour-coded by server index.
//!
//! Reads model snapshots only — no terminal I/O, no model mutation, no ingest
//! imports.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
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

/// Render the combined polar sky-plot into `area`.
///
/// Draws horizon / 30° / 60° rings, cardinal labels, and one labelled dot per
/// above-horizon satellite from every server.  When no plottable satellites
/// exist the grid is still drawn with a `no satellites` hint.
pub fn render_sky_plot(f: &mut Frame, area: Rect, model: &Model) {
    // Collect plottable satellites: (server_index, elevation, azimuth, used, prn, gnss_id).
    let mut sats: Vec<(usize, f64, f64, bool, i32, Option<String>)> = Vec::new();
    for server in model.servers() {
        if let Some(gps) = server.latest_gps() {
            for sat in &gps.satellites {
                let (Some(prn), Some(el), Some(az)) = (sat.prn, sat.elevation, sat.azimuth) else {
                    continue;
                };
                let used = sat.used == Some(true);
                sats.push((server.index, el, az, used, prn, sat.gnss_id.clone()));
            }
        }
    }
    let have_sats = !sats.is_empty();

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

            for (server_idx, el, az, used, prn, gnss_id) in &sats {
                let (x, y) = polar_to_xy(*el, *az);
                let color = server_color(*server_idx);
                let glyph = if *used { "●" } else { "○" };
                let label = sat_label(*prn, gnss_id.as_deref());
                let text = format!("{}{}", glyph, label);
                ctx.print(x, y, Line::styled(text, Style::default().fg(color)));
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
}
