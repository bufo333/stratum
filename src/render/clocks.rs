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
        .map(|age| format!("{:.1}s", age.as_secs_f64()))
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

pub fn render(f: &mut Frame, area: Rect, model: &Model, now: Instant) {
    let columns = if area.width >= 114 {
        3
    } else if area.width >= 76 {
        2
    } else {
        1
    };
    let rows = (model.server_count().max(1) as u16).div_ceil(columns);
    if area.width < 38 || area.height / rows < 7 {
        f.render_widget(
            Paragraph::new(format!(
                "Resize: {}x{} needed\n{} servers (including footer)",
                38 * columns,
                7 * rows + 1,
                model.server_count()
            )),
            area,
        );
        return;
    }
    let width = area.width / columns;
    let height = area.height / rows;
    for (index, server) in model.servers().iter().enumerate() {
        let index = index as u16;
        card(
            f,
            Rect::new(
                area.x + index % columns * width,
                area.y + index / columns * height,
                width,
                height,
            ),
            server,
            now,
        );
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
    let time = epoch.map(utc).unwrap_or_else(|| "--:--:--".into());
    let offset = server.timing_offset(now);
    let ntp_age = age_text(server.ntp_received_at(), now);
    let ntp_state = if server.status() == ServerStatus::Unreachable
        || server.status() == ServerStatus::Connecting
    {
        " OFFLINE"
    } else if server.ntp_received_at().is_some()
        && !is_fresh(server.status(), server.ntp_received_at(), now)
    {
        " STALE"
    } else {
        ""
    };
    let clock_age = age_text(sample.map(|s| s.received_at), now);
    let rtt = sample
        .map(|s| format!("{:.0}", s.rtt.as_secs_f64() * 1000.0))
        .unwrap_or_else(|| "n/a".into());
    let leap = server
        .latest_ntp()
        .and_then(|s| s.leap_status.as_deref())
        .unwrap_or("unknown");
    let mut lines: Vec<Line> = Vec::new();
    if area.height >= 12 {
        lines.push(Line::from(format!(
            "Server UTC ~ age {clock_age} RTT {rtt}ms"
        )));
        for row in segmented(&time) {
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
        lines.push(Line::from(format!(
            "Last offset {}{ntp_state}",
            offset_text(offset)
        )));
        lines.push(Line::from(format!("Light equiv {}", distance_text(offset))));
        lines.push(Line::from(format!("NTP age {ntp_age} Leap {leap}")));
    } else {
        lines.push(Line::styled(
            format!("Server UTC ~ {time} {clock_status}"),
            style,
        ));
        lines.push(Line::from(format!(
            "age {clock_age} RTT {rtt}ms | NTP {ntp_age}"
        )));
        lines.push(Line::styled(
            format!("-1us [{}] +1us (0 mid)", needle(offset, 11)),
            style,
        ));
        lines.push(Line::from(format!(
            "Last offset {}{ntp_state}",
            offset_text(offset)
        )));
        lines.push(Line::from(format!("Light equiv {}", distance_text(offset))));
        // Optional rows preserve leap details when the compact card has room.
        if inner.height >= 6 {
            lines.push(Line::from(format!("NTP age {ntp_age} Leap {leap}")));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClockSample, NtpProvenance, NtpSample, ServerUpdate};
    use crate::render::{draw, View};
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

    fn screen(model: &Model, width: u16, height: u16, now: Instant, view: View) -> String {
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
    fn synthetic_render_matrix() {
        let now = Instant::now();
        for n in [1, 3, 6] {
            for (width, height) in [(120, 40), (80, 24), (120, 24)] {
                let model = fixture(n, now);
                let output = screen(&model, width, height, now, View::Timing);
                for index in 0..n {
                    assert!(output.contains(&format!("synthetic-{index}")));
                }
                for label in [
                    "Server UTC ~",
                    "Last offset",
                    "Light equiv",
                    "us",
                    " m",
                    "RTT",
                    "age",
                    "Tab:",
                ] {
                    assert!(
                        output.contains(label),
                        "{n} {width}x{height}: {label}\n{output}"
                    );
                }
                // Run with --nocapture to inspect synthetic terminal buffers.
                println!("SYNTHETIC {n} hosts {width}x{height}\n{output}");
            }
        }
    }

    #[test]
    fn degraded_states_resize_and_dashboard() {
        let now = Instant::now();
        let mut model = fixture(1, now);
        let output = screen(&model, 40, 12, now, View::Timing);
        assert!(output.contains("Unsynchronised"));
        let stale = screen(&model, 40, 12, now + Duration::from_secs(6), View::Timing);
        assert!(stale.contains("STALE"));
        assert!(!stale.contains('^'));
        assert!(!stale.contains("SEC ["));
        assert!(stale.contains("6.0s"));
        model.apply(ServerUpdate::ClockResult {
            index: 0,
            sample: None,
        });
        let unavailable = screen(&model, 40, 12, now, View::Timing);
        assert!(unavailable.contains("unavailable"));
        assert!(unavailable.contains("+0.100 us"));
        model.apply(ServerUpdate::StatusChange {
            index: 0,
            status: ServerStatus::Unreachable,
        });
        assert!(screen(&model, 40, 12, now, View::Timing).contains("OFFLINE"));
        let resize = screen(&fixture(6, now), 80, 21, now, View::Timing);
        assert!(resize.contains("76x22"));
        for (w, h) in [(0, 0), (1, 1), (10, 2), (37, 24)] {
            let _ = screen(&model, w, h, now, View::Timing);
        }
        let long = Model::new(vec!["long-hostname-".repeat(20)]);
        assert!(screen(&long, 38, 8, now, View::Timing).contains("Server UTC ~"));
        let dashboard = screen(&fixture(3, now), 120, 40, now, View::Dashboard);
        for label in ["Host", "Offset", "GPS Fix", "synthetic-0", "Tab:"] {
            assert!(dashboard.contains(label));
        }
    }
}
