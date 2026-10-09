//! Timing instruments. All animation derives from explicit sample times.
use std::time::Instant;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
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

fn segmented(text: &str) -> [String; 3] {
    const DIGITS: [[&str; 3]; 10] = [
        [" _ ", "| |", "|_|"],
        ["   ", "  |", "  |"],
        [" _ ", " _|", "|_ "],
        [" _ ", " _|", " _|"],
        ["   ", "|_|", "  |"],
        [" _ ", "|_ ", " _|"],
        [" _ ", "|_ ", "|_|"],
        [" _ ", "  |", "  |"],
        [" _ ", "|_|", "|_|"],
        [" _ ", "|_|", " _|"],
    ];
    std::array::from_fn(|row| {
        text.chars()
            .map(|c| match c.to_digit(10) {
                Some(n) => DIGITS[n as usize][row],
                None if c == ':' => [" ", ".", "."][row],
                _ => ["   ", " _ ", "   "][row],
            })
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// Equal grid slots, including empty final slots; distribute remainder cells.
pub fn render(f: &mut Frame, area: Rect, model: &Model, columns: u16, now: Instant) {
    if columns == 0 || model.server_count() == 0 {
        return;
    }
    let rows = (model.server_count() as u16).div_ceil(columns);
    let large = area.height / rows >= 12;
    for (index, server) in model.servers().iter().enumerate() {
        let index = index as u16;
        let col = index % columns;
        let row = index / columns;
        let x = (u32::from(area.width) * u32::from(col) / u32::from(columns)) as u16;
        let right = (u32::from(area.width) * u32::from(col + 1) / u32::from(columns)) as u16;
        let y = (u32::from(area.height) * u32::from(row) / u32::from(rows)) as u16;
        let bottom = (u32::from(area.height) * u32::from(row + 1) / u32::from(rows)) as u16;
        card(
            f,
            Rect::new(area.x + x, area.y + y, right - x, bottom - y),
            server,
            now,
            large,
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

fn card(f: &mut Frame, area: Rect, server: &ServerState, now: Instant, large: bool) {
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
    let lines = if large {
        let mut lines = vec![Line::from(format!("UTC~ age{clock_age} RTT{rtt}"))];
        for row in segmented(&epoch.map(utc).unwrap_or_else(|| "--:--:--".into())) {
            lines.push(Line::styled(row, style.add_modifier(Modifier::BOLD)));
        }
        let sweep = match epoch {
            Some(ns) => {
                let position = ((ns % 60_000_000_000) as u128 * 24 / 60_000_000_000) as usize;
                let mut ticks = vec!['.'; 24];
                ticks[position] = '*';
                format!("SEC [{}]", ticks.into_iter().collect::<String>())
            }
            None => format!("--:--:-- {clock_status}"),
        };
        lines.push(Line::from(sweep));
        lines.push(Line::from("-1 us          0          +1 us"));
        lines.push(Line::styled(needle(offset, 30), style));
        lines.push(Line::from(format!("Last offset {last}")));
        lines.push(Line::from(format!(
            "Light equiv {}",
            compact_distance(offset)
        )));
        lines.push(Line::from(format!("NTP age {ntp_age}")));
        lines
    } else {
        vec![
            Line::styled(format!("UTC~{time} age{clock_age} RTT{rtt}"), style),
            Line::from(format!(
                "Last {last} Light equiv {}",
                compact_distance(offset)
            )),
            Line::styled(
                format!("-1us [{}] +1us NTP{ntp_age}", needle(offset, 7)),
                style,
            ),
        ]
    };
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClockSample, NtpProvenance, NtpSample, ServerUpdate};
    use crate::render::{draw, MiddleView};
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

    fn screen(model: &Model, width: u16, height: u16, now: Instant, view: MiddleView) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, model, view, now)).unwrap();
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
    fn synthetic_render_matrix_and_fixed_outer_rows() {
        let now = Instant::now();
        for n in 1..=6 {
            for (width, height) in [
                (120, 40),
                (120, 64),
                (120, 24),
                (80, 40),
                (80, 24),
                (119, 39),
                (121, 41),
            ] {
                let model = fixture(n, now);
                let [summary, middle, details] =
                    crate::render::dashboard_rows(Rect::new(0, 0, width, height));
                let (columns, min_width, min_height) = crate::render::timing_geometry(n, width);
                let fits = middle.width >= min_width && middle.height >= min_height;
                let large = middle.height / (n as u16).div_ceil(columns) >= 12;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|f| draw(f, &model, MiddleView::Graphs, now))
                    .unwrap();
                let graphs = terminal.backend().buffer().clone();
                terminal
                    .draw(|f| draw(f, &model, MiddleView::Timing, now))
                    .unwrap();
                let timing = terminal.backend().buffer();
                for region in [summary, details] {
                    for y in region.y..region.bottom() {
                        for x in region.x..region.right() {
                            assert_eq!(
                                graphs[(x, y)],
                                timing[(x, y)],
                                "{n} {width}x{height} outer row"
                            );
                        }
                    }
                }
                let output = screen(&model, width, height, now, MiddleView::Timing);
                assert_eq!(
                    output.matches("UTC~").count(),
                    if fits { n } else { 0 },
                    "{output}"
                );
                assert_eq!(
                    output.matches("SEC [").count(),
                    if fits && large { n } else { 0 },
                    "{output}"
                );
                assert_eq!(
                    output.matches("12:34:56").count(),
                    if fits && !large { n } else { 0 },
                    "{output}"
                );
                assert_eq!(output.contains("NTP Offset"), !fits, "{output}");
                assert_eq!(output.contains("NTP Frequency"), !fits, "{output}");
                assert_eq!(output.contains("Sky (all servers)"), !fits, "{output}");
                if fits {
                    for index in 0..n {
                        assert!(
                            output.contains(&format!(" {} synthetic-{index} ", index + 1)),
                            "{output}"
                        );
                    }
                    assert_eq!(output.matches("Light equiv").count(), n);
                    assert_eq!(output.matches("Leap Unsynchronised").count(), n);
                } else {
                    assert!(
                        output.contains(&format!("middle {min_width}x{min_height}")),
                        "{output}"
                    );
                }
                if (n == 3 || n == 6) && [(120, 40), (120, 64), (80, 24)].contains(&(width, height))
                {
                    for view in [MiddleView::Graphs, MiddleView::Timing] {
                        println!(
                            "SYNTHETIC {n} hosts {width}x{height} {view:?}\n{}",
                            screen(&model, width, height, now, view)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn approved_style_examples_and_uniform_grid() {
        let now = Instant::now();
        for (n, w, h, large, compact) in [
            (3, 120, 40, 3, 0),
            (6, 120, 40, 0, 6),
            (6, 120, 64, 6, 0),
            (3, 120, 24, 0, 3),
            (6, 120, 24, 0, 0),
            (3, 80, 40, 0, 3),
            (6, 80, 40, 0, 6),
            (3, 80, 24, 0, 0),
            (6, 80, 24, 0, 0),
        ] {
            let output = screen(&fixture(n, now), w, h, now, MiddleView::Timing);
            assert_eq!(output.matches("SEC [").count(), large, "{output}");
            assert_eq!(output.matches("12:34:56").count(), compact, "{output}");
        }
        // Two rows of 11 and 12 cells must both use the compact style.
        let model = fixture(6, now);
        let mut terminal = Terminal::new(TestBackend::new(119, 23)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &model, 3, now))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(text.matches("12:34:56").count(), 6);
        assert!(!text.contains("SEC ["));
        assert_eq!(buffer[(0, 22)].symbol(), "└");
        assert_eq!(buffer[(118, 22)].symbol(), "┘");
    }

    #[test]
    fn resize_recovery_long_names_and_tiny_areas() {
        let now = Instant::now();
        let model = fixture(6, now);
        let view = MiddleView::Timing;
        for (w, h, clocks) in [(80, 24, 0), (120, 64, 6), (80, 24, 0), (120, 40, 6)] {
            assert_eq!(
                screen(&model, w, h, now, view).matches("UTC~").count(),
                clocks
            );
        }
        for n in [0, 1, 6] {
            for (w, h) in [(0, 0), (1, 1), (10, 2), (37, 24)] {
                for view in [MiddleView::Graphs, MiddleView::Timing] {
                    let _ = screen(&fixture(n, now), w, h, now, view);
                }
            }
        }
        let long = Model::new(
            (0..5)
                .map(|i| format!("{i}-{}", "long-host-".repeat(20)))
                .collect(),
        );
        let output = screen(&long, 119, 64, now, view);
        assert_eq!(output.matches("UTC~").count(), 5);
        assert_eq!(output.matches("Leap unknown").count(), 5);
        // The unused final slot is retained rather than widening final cards.
        let middle = crate::render::dashboard_rows(Rect::new(0, 0, 119, 64))[1];
        for line in output
            .lines()
            .skip((middle.y + middle.height / 2) as usize)
            .take((middle.height - middle.height / 2) as usize)
        {
            assert!(line.chars().skip(119 * 2 / 3).all(|c| c == ' '), "{line}");
        }
    }

    fn card_screen(model: &Model, now: Instant, large: bool) -> String {
        let mut terminal = Terminal::new(TestBackend::new(38, if large { 12 } else { 5 })).unwrap();
        terminal
            .draw(|f| card(f, f.area(), &model.servers()[0], now, large))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..if large { 12 } else { 5 })
            .map(|y| (0..38).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn both_styles_quality_states_and_independent_recovery() {
        for large in [false, true] {
            let now = Instant::now();
            let mut model = fixture(1, now);
            let boundary = now + Duration::from_secs(5);
            let fresh = card_screen(&model, boundary, large);
            assert!(fresh.contains(if large { "SEC [" } else { "12:35:01" }));
            assert!(fresh.contains("+0.100us"));
            assert!(fresh.contains("Unsynchronised"));
            let stale = card_screen(&model, boundary + Duration::from_nanos(1), large);
            assert!(stale.contains(if large {
                "--:--:-- STALE"
            } else {
                "UTC~-- STALE"
            }));
            assert!(stale.contains(if large {
                "Last offset STALE"
            } else {
                "Last STALE"
            }));
            assert!(stale.contains("Light equiv n/a"));
            assert!(!stale.contains("SEC ["));
            assert!(!stale.contains('^'));
            model.apply(ServerUpdate::ClockResult {
                index: 0,
                sample: None,
            });
            let failed = card_screen(&model, now, large);
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
            let recovered = card_screen(&model, boundary + Duration::from_secs(1), large);
            assert!(recovered.contains(if large { "SEC [" } else { "00:00:01" }));
            assert!(recovered.contains(if large {
                "Last offset STALE"
            } else {
                "Last STALE"
            }));
            assert!(recovered.contains("age1.0s"));
            assert!(recovered.contains(if large { "NTP age 6.0s" } else { "NTP6.0s" }));
            for status in [
                ServerStatus::Unreachable,
                ServerStatus::Connecting,
                ServerStatus::Stale,
            ] {
                model.apply(ServerUpdate::StatusChange { index: 0, status });
                let output = card_screen(&model, now, large);
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
            let ntpq = card_screen(&model, now, large);
            assert!(ntpq.contains(if large { "Last offset n/a" } else { "Last n/a" }));
            assert!(ntpq.contains("Light equiv n/a"));
            assert!(ntpq.contains("Leap unknown"));
        }
    }

    #[test]
    fn both_styles_extremes_preserve_units_signs_and_markers() {
        for large in [false, true] {
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
                let output = card_screen(&model, now, large);
                let line = if large {
                    output.as_str()
                } else {
                    output.lines().nth(2).unwrap()
                };
                assert!(line.contains(&compact_offset(Some(value))), "{output}");
                assert!(line.contains(&compact_distance(Some(value))), "{output}");
                assert!(line.contains(if value < 0.0 { "<-" } else { ">+" }));
            }
            for value in [
                None,
                Some(f64::NAN),
                Some(f64::INFINITY),
                Some(f64::NEG_INFINITY),
            ] {
                let mut sample = model.servers()[0].latest_ntp().unwrap().clone();
                sample.offset_sec = value;
                model.apply(ServerUpdate::NtpSample {
                    index: 0,
                    received_at: now,
                    sample,
                });
                let output = card_screen(&model, now, large);
                assert!(output.contains("Light equiv n/a"));
                assert!(output.contains(if large { "Last offset n/a" } else { "Last n/a" }));
                assert!(!output.contains('^'));
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
            assert!(card_screen(&model, now, large).contains("RTT2e22ms"));
            let later = now + Duration::from_secs(1_000_000_000);
            let output = card_screen(&model, later, large);
            assert!(output.contains("age1e9s RTT2e22ms"), "{output}");
            assert!(output.contains(if large { "NTP age 1e9s" } else { "NTP1e9s" }));
            let long = Model::new(vec!["long-hostname-".repeat(20)]);
            let output = card_screen(&long, now, large);
            assert!(output.contains(if large {
                "--:--:-- OFFLINE"
            } else {
                "UTC~-- OFFLINE"
            }));
            assert!(output.contains("Leap unknown"));
        }
    }
}
