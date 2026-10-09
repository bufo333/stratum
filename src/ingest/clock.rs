//! Optional GNU-compatible date capture; unsupported output is unavailable.
use std::time::{Duration, Instant};

use openssh::Session;

use crate::model::ClockSample;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn capture(session: &Session) -> Option<ClockSample> {
    let start = Instant::now();
    let output = tokio::time::timeout(
        CAPTURE_TIMEOUT,
        super::ssh::run_command(session, "date", &["+%s.%N"]),
    )
    .await
    .ok()?
    .ok()?;
    normalize_capture(&output, start, Instant::now())
}

fn parse_epoch_ns(output: &str) -> Option<u64> {
    let (seconds, fraction) = output.trim().split_once('.')?;
    if seconds.is_empty()
        || !seconds.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() != 9
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    seconds
        .parse::<u64>()
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(fraction.parse::<u64>().ok()?)
}

fn normalize_capture(output: &str, start: Instant, end: Instant) -> Option<ClockSample> {
    let rtt = end.checked_duration_since(start)?;
    Some(ClockSample {
        epoch_ns: parse_epoch_ns(output)?,
        midpoint: start.checked_add(rtt / 2)?,
        received_at: end,
        rtt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_date_fixtures() {
        for (input, expected) in [
            ("0.000000000", 0),
            (" \t1.123456789\n", 1_123_456_789),
            ("18446744073.709551615", u64::MAX),
        ] {
            assert_eq!(parse_epoch_ns(input), Some(expected));
        }
        for input in [
            "",
            "1",
            "1.%N",
            "-1.000000000",
            "+1.000000000",
            "1.00000000",
            "1.0000000000",
            "1.000000000\n2.000000000",
            "NaN.000000000",
            "inf.000000000",
            "18446744073.709551616",
            "18446744074.000000000",
            "1. 00000000",
        ] {
            assert_eq!(parse_epoch_ns(input), None, "{input}");
        }
    }

    #[test]
    fn midpoint_projection_rollover_and_extremes() {
        let start = Instant::now();
        let end = start + Duration::from_secs(2);
        let s = normalize_capture("86399.000000000", start, end).unwrap();
        assert_eq!(s.midpoint, start + Duration::from_secs(1));
        assert_eq!(s.rtt, Duration::from_secs(2));
        assert_eq!(s.projected_ns(end), Some(86400_000_000_000));
        assert_eq!(s.projected_ns(start), None);
        assert_eq!(
            normalize_capture("0.000000000", start, start).unwrap().rtt,
            Duration::ZERO
        );
        assert!(normalize_capture("0.000000000", end, start).is_none());
        let large =
            normalize_capture("0.000000000", start, start + Duration::from_secs(100000)).unwrap();
        assert_eq!(
            large.projected_ns(large.received_at),
            Some(50000_000_000_000)
        );
        let max = normalize_capture("18446744073.709551615", start, start).unwrap();
        assert_eq!(max.projected_ns(start + Duration::from_nanos(1)), None);
    }
}
