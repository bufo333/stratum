//! Timing instruments. All animation derives from explicit sample times.
use std::time::Instant;

use ratatui::{
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::server_color;
use crate::model::{
    server::{is_fresh, sample_age},
    Model, ServerState, ServerStatus,
};

const C: f64 = 299_792_458.0;

fn light_distance(offset: Option<f64>) -> Option<f64> {
    offset
        .filter(|v| v.is_finite())
        .map(|v| v.abs() * C)
        .filter(|v| v.is_finite())
}

fn distance_text(offset: Option<f64>) -> String {
    match light_distance(offset) {
        Some(m) if m < 1000.0 => format!("{m:.3} m"),
        Some(m) => format!("{:.3} km", m / 1000.0),
        None => "n/a".into(),
    }
}

fn offset_text(offset: Option<f64>) -> String {
    match offset.filter(|v| v.is_finite()) {
        Some(v) => {
            let overflow = if v > 1e-6 {
                ">"
            } else if v < -1e-6 {
                "<"
            } else {
                ""
            };
            if v.abs() < 1.0 {
                format!("{overflow}{:+.3} us", v * 1e6)
            } else {
                format!("{overflow}{v:+.3e} s")
            }
        }
        None => "n/a".into(),
    }
}

fn needle(offset: Option<f64>, width: usize) -> String {
    let width = width.max(3);
    let mut cells = vec!['-'; width];
    cells[width / 2] = '|';
    if let Some(value) = offset.filter(|v| v.is_finite()) {
        let position =
            (((value / 1e-6).clamp(-1.0, 1.0) + 1.0) / 2.0 * (width - 1) as f64).round() as usize;
        cells[position] = if value > 1e-6 {
            '>'
        } else if value < -1e-6 {
            '<'
        } else {
            '^'
        };
    }
    cells.into_iter().collect()
}

fn age_text(at: Option<Instant>, now: Instant) -> String {
    sample_age(at, now)
        .map(|age| format!("{}s", compact_number(age.as_secs_f64(), 1, 4)))
        .unwrap_or_else(|| "n/a".into())
}

fn utc(epoch_ns: u64) -> String {
    let seconds = epoch_ns / 1_000_000_000 % 86400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Fixed five-row cards; unused slots in the last row retain equal widths.
pub fn render(f: &mut Frame, area: Rect, model: &Model, columns: u16, now: Instant) {
    if columns == 0 {
        return;
    }
    let width = area.width / columns;
    for (index, server) in model.servers().iter().enumerate() {
        let index = index as u16;
        card(
            f,
            Rect::new(
                area.x + index % columns * width,
                area.y + index / columns * 5,
                width,
                5,
            ),
            server,
            now,
        );
    }
}

// Bound numbers before composing lines so signs, units and status never clip.
fn compact_number(value: f64, precision: usize, budget: usize) -> String {
    let fixed = format!("{value:.precision$}");
    if fixed.len() <= budget {
        fixed
    } else {
        format!("{value:.0e}")
    }
}

fn compact_offset(offset: Option<f64>) -> String {
    let full = offset_text(offset).replace(' ', "");
    if full.len() <= 10 {
        return full;
    }
    match offset.filter(|v| v.is_finite()) {
        Some(v) => format!("{}{v:+.0e}s", if v < 0.0 { "<" } else { ">" }),
        None => "n/a".into(),
    }
}

fn compact_distance(offset: Option<f64>) -> String {
    let full = distance_text(offset).replace(' ', "");
    if full.len() <= 9 {
        return full;
    }
    match light_distance(offset) {
        Some(m) => format!("{}km", compact_number(m / 1000.0, 3, 7)),
        None => "n/a".into(),
    }
}

fn card(f: &mut Frame, area: Rect, server: &ServerState, now: Instant) {
    let color = server_color(server.index);
    let style = Style::default().fg(color);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(format!(" {} {} ", server.index + 1, server.hostname))
        .title_bottom(format!(
            " Leap {} ",
            server
                .latest_ntp()
                .and_then(|s| s.leap_status.as_deref())
                .unwrap_or("unknown")
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let sample = server.clock();
    let fresh = is_fresh(server.status(), sample.map(|s| s.received_at), now);
    let epoch = sample.filter(|_| fresh).and_then(|s| s.projected_ns(now));
    let clock_status = match server.status() {
        ServerStatus::Unreachable => "OFFLINE",
        ServerStatus::Connecting => "OFFLINE",
        ServerStatus::Stale => "STALE",
        ServerStatus::Ok if sample.is_none() => "unavailable",
        ServerStatus::Ok if epoch.is_none() => "STALE",
        ServerStatus::Ok => "",
    };
    let time = epoch
        .map(utc)
        .unwrap_or_else(|| format!("-- {clock_status}"));
    let offset = server.timing_offset(now);
    let ntp_age = age_text(server.ntp_received_at(), now);
    let ntp_state = match server.status() {
        ServerStatus::Connecting | ServerStatus::Unreachable => "OFFLINE",
        ServerStatus::Stale => "STALE",
        ServerStatus::Ok
            if server.ntp_received_at().is_some()
                && !is_fresh(server.status(), server.ntp_received_at(), now) =>
        {
            "STALE"
        }
        _ => "",
    };
    let clock_age = age_text(sample.map(|s| s.received_at), now);
    let rtt = sample
        .map(|s| format!("{}ms", compact_number(s.rtt.as_secs_f64() * 1000.0, 0, 4)))
        .unwrap_or_else(|| "n/a".into());
    let last = if ntp_state.is_empty() {
        compact_offset(offset)
    } else {
        ntp_state.into()
    };
    let lines = vec![
        Line::styled(format!("UTC~{time} age{clock_age} RTT{rtt}"), style),
        Line::from(format!(
            "Last {last} Light equiv {}",
            compact_distance(offset)
        )),
        Line::styled(
            format!("-1us [{}] +1us NTP{ntp_age}", needle(offset, 7)),
            style,
        ),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClockSample, NtpProvenance, NtpSample, ServerUpdate};
    use crate::render::draw;
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::Duration;

    fn fixture(n: usize, now: Instant) -> Model {
        let mut model = Model::new((0..n).map(|i| format!("synthetic-{i}")).collect());
        for index in 0..n {
            model.apply(ServerUpdate::NtpSample {
                index,
                received_at: now,
                sample: NtpSample {
                    provenance: NtpProvenance::ChronyTracking,
                    offset_sec: Some(0.0000001),
                    rms_offset_sec: None,
                    freq_ppm: None,
                    stratum: Some(1),
                    reference: None,
                    root_delay_sec: None,
                    root_dispersion_sec: None,
                    leap_status: Some("Unsynchronised".into()),
                    sources: vec![],
                },
            });
            model.apply(ServerUpdate::ClockResult {
                index,
                sample: Some(ClockSample {
                    epoch_ns: 45_296_000_000_000,
                    midpoint: now,
                    received_at: now,
                    rtt: Duration::from_millis(2),
                }),
            });
        }
        model
    }

    fn screen(model: &Model, width: u16, height: u16, now: Instant) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, model, now)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn numeric_instruments() {
        assert_eq!(utc(86400_000_000_000), "00:00:00");
        assert_eq!(utc(86399_999_999_999), "23:59:59");
        assert!((light_distance(Some(1e-6)).unwrap() - 299.792458).abs() < 1e-9);
        assert_eq!(distance_text(Some(0.0)), "0.000 m");
        assert_eq!(distance_text(Some(-1e-6)), "299.792 m");
        assert_eq!(distance_text(Some(1000.0 / C)), "1.000 km");
        assert_eq!(distance_text(Some(999.0 / C)), "999.000 m");
        for value in [None, Some(f64::NAN), Some(f64::INFINITY), Some(f64::MAX)] {
            assert_eq!(distance_text(value), "n/a");
        }
        for value in [0.0, 1e-7, -1e-7, 1e-6, -1e-6] {
            assert!(needle(Some(value), 21).contains('^'));
            assert!(!offset_text(Some(value)).starts_with(['<', '>']));
        }
        assert_eq!(offset_text(Some(1e-7)), "+0.100 us");
        assert_eq!(offset_text(Some(-1e-7)), "-0.100 us");
        assert!(needle(Some(2e-6), 21).ends_with('>'));
        assert!(needle(Some(-2e-6), 21).starts_with('<'));
        assert_eq!(offset_text(Some(2e-6)), ">+2.000 us");
        assert_eq!(offset_text(Some(-2e-6)), "<-2.000 us");
        for value in [None, Some(f64::NAN), Some(f64::INFINITY)] {
            assert!(!needle(value, 21).contains('^'));
            assert_eq!(offset_text(value), "n/a");
        }
    }

    #[test]
    fn synthetic_render_matrix() {
        let now = Instant::now();
        for (n, width, height) in [
            (1, 120, 24),
            (1, 120, 40),
            (3, 120, 24),
            (3, 120, 40),
            (6, 120, 40),
            (3, 80, 29),
            (6, 80, 37),
            (2, 79, 26),
            (4, 119, 34),
            (5, 121, 34),
        ] {
            let output = screen(&fixture(n, now), width, height, now);
            assert_eq!(output.matches("UTC~").count(), n, "{output}");
            assert_eq!(output.matches("Light equiv").count(), n, "{output}");
            for label in [
                "Server Summary",
                "NTP Offset",
                "NTP Frequency",
                "Sky (all servers)",
                "NTP/Chrony",
                "GPS",
                "Unsynchronised",
                "q: quit",
            ] {
                assert!(
                    output.contains(label),
                    "{n} {width}x{height}: {label}\n{output}"
                );
            }
            assert!(!output.contains("Tab:"));
            println!("SYNTHETIC {n} hosts {width}x{height}\n{output}");
        }
    }

    #[test]
    fn long_names_keep_equal_slots_and_fixed_card_height() {
        let now = Instant::now();
        let model = Model::new(
            (0..5)
                .map(|i| format!("{i}-{}", "long-host-".repeat(20)))
                .collect(),
        );
        for height in [32, 40, 60] {
            let output = screen(&model, 119, height, now);
            let rows: Vec<_> = output.lines().collect();
            assert_eq!(output.matches("UTC~").count(), 5);
            assert_eq!(output.matches("Leap unknown").count(), 5);
            // Summary is 8 rows, then two 5-row strips. Extra height goes below.
            assert_eq!(rows[9].matches("UTC~").count(), 3);
            assert_eq!(rows[14].matches("UTC~").count(), 2);
            assert!(rows[18].contains("NTP Offset"));
            assert!(rows[14].chars().skip(78).all(|c| c == ' '));
        }
    }

    #[test]
    fn dashboard_first_fallback_and_tiny_areas() {
        let now = Instant::now();
        for (n, w, h, hint) in [
            (3, 80, 24, "76x29"),
            (6, 80, 24, "76x37"),
            (6, 120, 24, "114x32"),
        ] {
            let output = screen(&fixture(n, now), w, h, now);
            assert!(output.contains(hint), "{output}");
            assert!(!output.contains("UTC~"));
            for label in [
                "Server Summary",
                "NTP Offset",
                "Sky (all servers)",
                "NTP/Chrony",
                "GPS",
            ] {
                assert!(output.contains(label), "{label}\n{output}");
            }
            println!("SYNTHETIC fallback {n} hosts {w}x{h}\n{output}");
        }
        for n in [0, 1, 6] {
            for (w, h) in [(0, 0), (1, 1), (10, 2), (37, 24)] {
                let _ = screen(&fixture(n, now), w, h, now);
            }
        }
    }

    fn card_screen(model: &Model, now: Instant) -> String {
        let mut terminal = Terminal::new(TestBackend::new(38, 5)).unwrap();
        terminal
            .draw(|f| card(f, f.area(), &model.servers()[0], now))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..5)
            .map(|y| (0..38).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn compact_quality_states_and_independent_recovery() {
        let now = Instant::now();
        let mut model = fixture(1, now);
        let boundary = now + Duration::from_secs(5);
        let fresh = card_screen(&model, boundary);
        assert!(fresh.contains("12:35:01"));
        assert!(fresh.contains("+0.100us"));
        assert!(fresh.contains("Unsynchronised"));
        let stale = card_screen(&model, boundary + Duration::from_nanos(1));
        assert!(stale.contains("UTC~-- STALE"));
        assert!(stale.contains("Last STALE Light equiv n/a"));
        assert!(!stale.contains('^'));
        model.apply(ServerUpdate::ClockResult {
            index: 0,
            sample: None,
        });
        let failed = card_screen(&model, now);
        assert!(failed.contains("unavailable"));
        assert!(failed.contains("+0.100us"));
        model.apply(ServerUpdate::ClockResult {
            index: 0,
            sample: Some(ClockSample {
                epoch_ns: 0,
                midpoint: boundary,
                received_at: boundary,
                rtt: Duration::from_millis(1),
            }),
        });
        let recovered = card_screen(&model, boundary + Duration::from_secs(1));
        assert!(recovered.contains("00:00:01"));
        assert!(recovered.contains("Last STALE"));
        assert!(recovered.contains("age1.0s"));
        assert!(recovered.contains("NTP6.0s"));
        for status in [
            ServerStatus::Unreachable,
            ServerStatus::Connecting,
            ServerStatus::Stale,
        ] {
            model.apply(ServerUpdate::StatusChange { index: 0, status });
            let output = card_screen(&model, now);
            assert!(output.contains(if status == ServerStatus::Stale {
                "STALE"
            } else {
                "OFFLINE"
            }));
            assert!(!output.contains('^'));
        }
        let mut sample = model.servers()[0].latest_ntp().unwrap().clone();
        sample.provenance = NtpProvenance::NtpqPeer;
        sample.leap_status = None;
        model.apply(ServerUpdate::StatusChange {
            index: 0,
            status: ServerStatus::Unreachable,
        });
        model.apply(ServerUpdate::NtpSample {
            index: 0,
            received_at: now,
            sample,
        });
        let ntpq = card_screen(&model, now);
        assert!(ntpq.contains("Last n/a Light equiv n/a"));
        assert!(ntpq.contains("Leap unknown"));
    }

    #[test]
    fn compact_extremes_preserve_units_signs_and_markers() {
        let now = Instant::now();
        let mut model = fixture(1, now);
        for value in [2e-6, -2e-6, 0.999999, -0.999999, 1e100, -1e100, f64::MAX] {
            let mut sample = model.servers()[0].latest_ntp().unwrap().clone();
            sample.offset_sec = Some(value);
            model.apply(ServerUpdate::NtpSample {
                index: 0,
                received_at: now,
                sample,
            });
            let output = card_screen(&model, now);
            let line = output.lines().nth(2).unwrap();
            assert!(line.contains(&compact_offset(Some(value))), "{output}");
            assert!(line.contains(&compact_distance(Some(value))), "{output}");
            assert!(line.contains(if value < 0.0 { "<-" } else { ">+" }));
        }
        model.apply(ServerUpdate::ClockResult {
            index: 0,
            sample: Some(ClockSample {
                epoch_ns: 0,
                midpoint: now,
                received_at: now,
                rtt: Duration::MAX,
            }),
        });
        assert!(card_screen(&model, now).contains("RTT2e22ms"));
        let later = now + Duration::from_secs(1_000_000_000);
        let output = card_screen(&model, later);
        assert!(output.contains("age1e9s RTT2e22ms"), "{output}");
        assert!(output.contains("NTP1e9s"));
        let long = Model::new(vec!["long-hostname-".repeat(20)]);
        let output = card_screen(&long, now);
        assert!(output.contains("UTC~-- OFFLINE"));
        assert!(output.contains("Leap unknown"));
    }
}
