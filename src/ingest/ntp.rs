//! NTP/chrony telemetry collection and parsing.
//!
//! Attempts to run `chronyc tracking` and `chronyc sources -v` via SSH.
//! Falls back to `ntpq -p` if chronyc is not found.
//!
//! **Field inventory note (UNVERIFIED against live output):** the field
//! structs in `model::server` use `Option<T>` for all telemetry fields.
//! Parsing here maps only the fields that are present in the fixture strings
//! used in the test suite.  Any field that cannot be confirmed from parsing
//! logic or the fixture strings is left as `None`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use openssh::Session;
use tokio::sync::mpsc;

use crate::model::{NtpSample, NtpSource, ServerUpdate};

/// Run the NTP polling loop for one server.
///
/// Polls every `interval`, sends `ServerUpdate::NtpSample` on success.
/// Returns `Err` to signal the ingest restart loop that a reconnect is needed.
/// Returns `Ok(())` when the channel closes (shutdown).
pub async fn run_ntp_loop(
    index: usize,
    hostname: &str,
    session: &Arc<Session>,
    interval: Duration,
    tx: mpsc::Sender<ServerUpdate>,
) -> Result<()> {
    loop {
        match collect_ntp(hostname, session).await {
            Ok(sample) => {
                if tx
                    .send(ServerUpdate::NtpSample { index, sample })
                    .await
                    .is_err()
                {
                    return Ok(()); // channel closed — shutdown
                }
            }
            Err(e) => {
                return Err(e.context(format!("NTP collection from {hostname}")));
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Collect one NTP sample from `hostname` via SSH.
///
/// Tries `chronyc -c tracking` first (CSV, stable parsing), then
/// `chronyc tracking`, then falls back to `ntpq -p`.
async fn collect_ntp(hostname: &str, session: &Arc<Session>) -> Result<NtpSample> {
    // 1. Try chronyc CSV tracking.
    if let Ok(csv) = super::ssh::run_command(session, "chronyc", &["-c", "tracking"]).await {
        if let Ok(sample) = parse_chronyc_csv_tracking(&csv) {
            // Also fetch sources.
            let sources = if let Ok(src_out) =
                super::ssh::run_command(session, "chronyc", &["-c", "sources"]).await
            {
                parse_chronyc_csv_sources(&src_out).unwrap_or_default()
            } else {
                vec![]
            };
            return Ok(NtpSample { sources, ..sample });
        }
    }

    // 2. Try chronyc verbose tracking.
    if let Ok(out) = super::ssh::run_command(session, "chronyc", &["tracking"]).await {
        if let Ok(sample) = parse_chronyc_tracking(&out) {
            let sources = if let Ok(src_out) =
                super::ssh::run_command(session, "chronyc", &["sources", "-v"]).await
            {
                parse_chronyc_sources_v(&src_out).unwrap_or_default()
            } else {
                vec![]
            };
            return Ok(NtpSample { sources, ..sample });
        }
    }

    // 3. Fall back to ntpq.
    let ntpq_out = super::ssh::run_command(session, "ntpq", &["-p"])
        .await
        .with_context(|| format!("Neither chronyc nor ntpq responded on {hostname}"))?;
    parse_ntpq_p(&ntpq_out)
        .with_context(|| format!("Failed to parse ntpq -p output from {hostname}"))
}

// ---------------------------------------------------------------------------
// Parsing: chronyc -c tracking (CSV)
// ---------------------------------------------------------------------------
//
// chronyc -c tracking produces a single CSV line.
// Confirmed field order from chrony source (chronyc.c, csv_tracking):
//   0  Reference ID (hex)
//   1  Reference name/IP
//   2  Stratum
//   3  Ref time (as float seconds since epoch — not used)
//   4  System time offset (seconds, positive = slow)
//   5  Last offset
//   6  RMS offset
//   7  Frequency (ppm)
//   8  Residual freq (ppm)
//   9  Skew (ppm)
//  10  Root delay (seconds)
//  11  Root dispersion (seconds)
//  12  Update interval (seconds)
//  13  Leap status (0=Normal, 1=InsertSecond, 2=DeleteSecond, 3=Unsynchronised)
//
// NOTE: This field order is UNVERIFIED against a live server.  All fields are
// parsed as Option<T>; unparseable values produce None.

/// Parse one line of `chronyc -c tracking` CSV output.
pub fn parse_chronyc_csv_tracking(input: &str) -> Result<NtpSample> {
    let line = input.trim();
    if line.is_empty() {
        anyhow::bail!("empty chronyc -c tracking output");
    }
    let fields: Vec<&str> = line.split(',').collect();

    let reference = fields
        .get(1)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let stratum = fields.get(2).and_then(|s| s.trim().parse::<u8>().ok());
    let offset_sec = fields.get(5).and_then(|s| s.trim().parse::<f64>().ok());
    let rms_offset_sec = fields.get(6).and_then(|s| s.trim().parse::<f64>().ok());
    let freq_ppm = fields.get(7).and_then(|s| s.trim().parse::<f64>().ok());
    let root_delay_sec = fields.get(10).and_then(|s| s.trim().parse::<f64>().ok());
    let root_dispersion_sec = fields.get(11).and_then(|s| s.trim().parse::<f64>().ok());
    let leap_status = fields.get(13).map(|s| match s.trim() {
        "0" => "Normal".to_owned(),
        "1" => "InsertSecond".to_owned(),
        "2" => "DeleteSecond".to_owned(),
        "3" => "Unsynchronised".to_owned(),
        other => other.to_owned(),
    });

    Ok(NtpSample {
        offset_sec,
        rms_offset_sec,
        freq_ppm,
        stratum,
        reference,
        root_delay_sec,
        root_dispersion_sec,
        leap_status,
        sources: vec![],
    })
}

/// Parse one line of `chronyc -c sources` CSV output.
/// Each line represents one source.
/// Field order (from chrony source, chronyc.c, csv_sources):
///   0  Mode (^=server, ==peer, #=local ref)
///   1  State (*=selected, +=combined, -=not combined, ?=unreachable, x=falseticker, ~=stale)
///   2  Name/IP
///   3  Stratum
///   4  Poll (exponent)
///   5  Reach (octal)
///   6  Last Rx (seconds)
///   7  Last sample offset (seconds)
///   8  Last sample error margin (seconds)
///   9  Adjusted offset (seconds)
///  10  Adjusted error margin (seconds)
///
/// NOTE: UNVERIFIED against live output.
pub fn parse_chronyc_csv_sources(input: &str) -> Result<Vec<NtpSource>> {
    let mut sources = Vec::new();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let mode = fields.first().map(|s| s.trim()).unwrap_or("");
        let state = fields.get(1).map(|s| s.trim()).unwrap_or("");
        let flags = if mode.is_empty() && state.is_empty() {
            None
        } else {
            Some(format!("{mode}{state}"))
        };
        let name = fields
            .get(2)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let stratum = fields.get(3).and_then(|s| s.trim().parse::<u8>().ok());
        let poll = fields.get(4).and_then(|s| s.trim().parse::<i32>().ok());
        let reach = fields
            .get(5)
            .and_then(|s| u8::from_str_radix(s.trim(), 8).ok());
        let last_rx_sec = fields.get(6).and_then(|s| s.trim().parse::<i64>().ok());
        let offset_sec = fields.get(9).and_then(|s| s.trim().parse::<f64>().ok());

        sources.push(NtpSource {
            name,
            stratum,
            poll,
            reach,
            last_rx_sec,
            offset_sec,
            flags,
        });
    }
    Ok(sources)
}

// ---------------------------------------------------------------------------
// Parsing: chronyc tracking (verbose, key: value format)
// ---------------------------------------------------------------------------

/// Parse `chronyc tracking` human-readable output.
///
/// Example:
/// ```text
/// Reference ID    : A9FEA97B (169.254.169.123)
/// Stratum         : 4
/// ...
/// System time     : 0.000000123 seconds fast of NTP time
/// Last offset     : +0.000000045 seconds
/// RMS offset      : 0.000000012 seconds
/// Frequency       : 5.123 ppm fast
/// Residual freq   : +0.000 ppm
/// Skew            : 0.010 ppm
/// Root delay      : 0.001000000 seconds
/// Root dispersion : 0.002000000 seconds
/// Update interval : 64.0 seconds
/// Leap status     : Normal
/// ```
///
/// NOTE: field names and format UNVERIFIED against live output.
pub fn parse_chronyc_tracking(input: &str) -> Result<NtpSample> {
    let mut reference: Option<String> = None;
    let mut stratum: Option<u8> = None;
    let mut offset_sec: Option<f64> = None;
    let mut rms_offset_sec: Option<f64> = None;
    let mut freq_ppm: Option<f64> = None;
    let mut root_delay_sec: Option<f64> = None;
    let mut root_dispersion_sec: Option<f64> = None;
    let mut leap_status: Option<String> = None;

    for line in input.lines() {
        let line = line.trim();
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            let val = val.trim();
            match key.as_str() {
                "reference id" => {
                    // e.g. "A9FEA97B (169.254.169.123)"
                    let name = val
                        .split_once('(')
                        .and_then(|(_, rest)| rest.split_once(')'))
                        .map(|(name, _)| name.trim().to_owned())
                        .unwrap_or_else(|| val.to_owned());
                    reference = Some(name);
                }
                "stratum" => {
                    stratum = val.parse::<u8>().ok();
                }
                "last offset" => {
                    // e.g. "+0.000000045 seconds"
                    offset_sec = val
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<f64>().ok());
                }
                "rms offset" => {
                    rms_offset_sec = val
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<f64>().ok());
                }
                "frequency" => {
                    // e.g. "5.123 ppm fast" or "-3.1 ppm slow"
                    let first = val
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<f64>().ok());
                    let direction = val.to_lowercase();
                    freq_ppm = first.map(|v| if direction.contains("slow") { -v } else { v });
                }
                "root delay" => {
                    root_delay_sec = val
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<f64>().ok());
                }
                "root dispersion" => {
                    root_dispersion_sec = val
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<f64>().ok());
                }
                "leap status" => {
                    leap_status = Some(val.to_owned());
                }
                _ => {}
            }
        }
    }

    // Require at least stratum to consider the parse successful.
    if stratum.is_none() && reference.is_none() {
        anyhow::bail!("chronyc tracking output did not contain recognized fields");
    }

    Ok(NtpSample {
        offset_sec,
        rms_offset_sec,
        freq_ppm,
        stratum,
        reference,
        root_delay_sec,
        root_dispersion_sec,
        leap_status,
        sources: vec![],
    })
}

/// Parse `chronyc sources -v` output.
///
/// Example lines (after the header):
/// ```text
/// ^* ntp1.example.com          2   7   377   16  +15us[+18us] +/-  450us
/// ^- ntp2.example.com          2   7   377   46   -84us[-81us] +/-  572us
/// ```
///
/// NOTE: format UNVERIFIED against live output.
pub fn parse_chronyc_sources_v(input: &str) -> Result<Vec<NtpSource>> {
    let mut sources = Vec::new();
    for line in input.lines() {
        let line = line.trim();
        // Skip header/blank lines: those starting with '=' or not with '^'/'=' prefix.
        if line.is_empty()
            || line.starts_with('=')
            || line.starts_with("210")
            || line.starts_with("MS")
        {
            continue;
        }
        // Lines of interest start with "^" or "=" (mode character).
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < 3 {
            continue;
        }
        let mode_char = chars[0];
        if mode_char != '^' && mode_char != '=' && mode_char != '#' {
            continue;
        }
        let state_char = chars[1];
        let flags = Some(format!("{mode_char}{state_char}"));
        let rest = line[2..].trim();
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let name = parts[0].to_owned();
        let stratum = parts.get(1).and_then(|s| s.parse::<u8>().ok());
        let poll = parts.get(2).and_then(|s| s.parse::<i32>().ok());
        let reach = parts.get(3).and_then(|s| u8::from_str_radix(s, 8).ok());
        let last_rx_sec = parts.get(4).and_then(|s| s.parse::<i64>().ok());
        // Offset field: e.g. "+15us[+18us]"  — parse the first number in microseconds.
        let offset_sec = parts.get(5).and_then(|s| parse_chrony_time_offset(s));

        sources.push(NtpSource {
            name,
            stratum,
            poll,
            reach,
            last_rx_sec,
            offset_sec,
            flags,
        });
    }
    Ok(sources)
}

/// Parse chrony time offsets like "+15us", "-84us", "+0.000123s", etc.
/// Returns the value in seconds, or `None` if unparseable.
fn parse_chrony_time_offset(s: &str) -> Option<f64> {
    // Take the first token before '[', then strip unit suffix.
    let token = s.split('[').next()?;
    let token = token.trim_start_matches('+');
    if let Some(micros) = token.strip_suffix("us") {
        micros.parse::<f64>().ok().map(|v| v / 1_000_000.0)
    } else if let Some(millis) = token.strip_suffix("ms") {
        millis.parse::<f64>().ok().map(|v| v / 1_000.0)
    } else if let Some(ns) = token.strip_suffix("ns") {
        ns.parse::<f64>().ok().map(|v| v / 1_000_000_000.0)
    } else if let Some(secs) = token.strip_suffix('s') {
        secs.parse::<f64>().ok()
    } else {
        token.parse::<f64>().ok()
    }
}

// ---------------------------------------------------------------------------
// Parsing: ntpq -p (table format)
// ---------------------------------------------------------------------------

/// Parse `ntpq -p` tabular output into an `NtpSample`.
///
/// Columns: remote, refid, st, t, when, poll, reach, delay, offset, jitter
///
/// NOTE: column positions UNVERIFIED against live output.
pub fn parse_ntpq_p(input: &str) -> Result<NtpSample> {
    let mut sources = Vec::new();
    let mut selected_offset: Option<f64> = None;
    let mut selected_stratum: Option<u8> = None;
    let mut selected_ref: Option<String> = None;

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('=') || line.starts_with("remote") {
            continue;
        }
        // Status prefix: '*' = selected, '+' = candidate, '-' = discarded, 'o' = PPS.
        let (flags, rest) = if let Some(first) = line.chars().next() {
            if "*+-ox#".contains(first) {
                (Some(first.to_string()), &line[1..])
            } else {
                (None, line)
            }
        } else {
            continue;
        };

        let parts: Vec<&str> = rest.split_whitespace().collect();
        // Expected: remote refid st t when poll reach delay offset jitter
        //            0      1     2  3 4    5    6     7     8      9
        if parts.len() < 8 {
            continue;
        }
        let name = parts[0]
            .trim_start_matches('+')
            .trim_start_matches('-')
            .to_owned();
        let stratum = parts.get(2).and_then(|s| s.parse::<u8>().ok());
        let last_rx_sec = parts.get(4).and_then(|s| s.parse::<i64>().ok());
        let poll = parts.get(5).and_then(|s| s.parse::<i32>().ok());
        let reach = parts.get(6).and_then(|s| u8::from_str_radix(s, 8).ok());
        // ntpq reports offset in milliseconds.
        let offset_sec = parts
            .get(8)
            .and_then(|s| s.parse::<f64>().ok())
            .map(|ms| ms / 1000.0);

        if flags.as_deref() == Some("*") {
            selected_offset = offset_sec;
            selected_stratum = stratum;
            selected_ref = Some(name.clone());
        }

        sources.push(NtpSource {
            name,
            stratum,
            poll,
            reach,
            last_rx_sec,
            offset_sec,
            flags,
        });
    }

    Ok(NtpSample {
        offset_sec: selected_offset,
        rms_offset_sec: None,
        freq_ppm: None,
        stratum: selected_stratum,
        reference: selected_ref,
        root_delay_sec: None,
        root_dispersion_sec: None,
        leap_status: None,
        sources,
    })
}

// ---------------------------------------------------------------------------
// Tests (Step 9 fixtures)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- chronyc -c tracking CSV fixture ---
    // Fabricated from the chrony source field order (UNVERIFIED against live).
    const CHRONYC_CSV_TRACKING: &str =
        "A9FEA97B,169.254.169.123,4,1704067200.0,0.000000050,+0.000000123,0.000000045,-5.200,+0.001,0.010,0.001000,0.002000,64.0,0\n";

    #[test]
    fn parse_chronyc_csv_tracking_ok() {
        let s = parse_chronyc_csv_tracking(CHRONYC_CSV_TRACKING).unwrap();
        assert_eq!(s.reference.as_deref(), Some("169.254.169.123"));
        assert_eq!(s.stratum, Some(4));
        assert!((s.offset_sec.unwrap() - 0.000000123).abs() < 1e-12);
        assert!((s.rms_offset_sec.unwrap() - 0.000000045).abs() < 1e-12);
        assert!((s.freq_ppm.unwrap() - (-5.200)).abs() < 1e-6);
        assert!((s.root_delay_sec.unwrap() - 0.001000).abs() < 1e-9);
        assert!((s.root_dispersion_sec.unwrap() - 0.002000).abs() < 1e-9);
        assert_eq!(s.leap_status.as_deref(), Some("Normal"));
    }

    #[test]
    fn parse_chronyc_csv_tracking_empty_fails() {
        assert!(parse_chronyc_csv_tracking("").is_err());
    }

    // --- chronyc -c sources CSV fixture ---
    const CHRONYC_CSV_SOURCES: &str = "\
^,*,169.254.169.123,2,7,377,16,+0.000015,0.000002,+0.000018,0.000003\n\
^,-,ntp.example.com,3,7,377,46,-0.000084,0.000005,-0.000081,0.000006\n";

    #[test]
    fn parse_chronyc_csv_sources_ok() {
        let sources = parse_chronyc_csv_sources(CHRONYC_CSV_SOURCES).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name, "169.254.169.123");
        assert_eq!(sources[0].stratum, Some(2));
        assert_eq!(sources[0].flags.as_deref(), Some("^*"));
        assert!((sources[0].offset_sec.unwrap() - 0.000018).abs() < 1e-9);
        assert_eq!(sources[1].name, "ntp.example.com");
        assert_eq!(sources[1].flags.as_deref(), Some("^-"));
    }

    // --- chronyc tracking verbose fixture ---
    const CHRONYC_TRACKING_VERBOSE: &str = "\
Reference ID    : A9FEA97B (169.254.169.123)
Stratum         : 4
Ref time (UTC)  : Mon Jan 01 00:00:00 2024
System time     : 0.000000050 seconds fast of NTP time
Last offset     : +0.000000123 seconds
RMS offset      : 0.000000045 seconds
Frequency       : 5.200 ppm slow
Residual freq   : +0.001 ppm
Skew            : 0.010 ppm
Root delay      : 0.001000000 seconds
Root dispersion : 0.002000000 seconds
Update interval : 64.0 seconds
Leap status     : Normal
";

    #[test]
    fn parse_chronyc_tracking_verbose_ok() {
        let s = parse_chronyc_tracking(CHRONYC_TRACKING_VERBOSE).unwrap();
        assert_eq!(s.reference.as_deref(), Some("169.254.169.123"));
        assert_eq!(s.stratum, Some(4));
        assert!((s.offset_sec.unwrap() - 0.000000123).abs() < 1e-12);
        assert!((s.rms_offset_sec.unwrap() - 0.000000045).abs() < 1e-12);
        // "5.200 ppm slow" → -5.200
        assert!((s.freq_ppm.unwrap() - (-5.200)).abs() < 1e-6);
        assert_eq!(s.leap_status.as_deref(), Some("Normal"));
    }

    // --- chronyc sources -v fixture ---
    const CHRONYC_SOURCES_V: &str = "\
210 Number of sources = 2
MS Name/IP address         Stratum Poll Reach LastRx Last sample
===============================================================================
^* 169.254.169.123               2    7   377     16   +15us[+18us] +/-  450us
^- ntp.example.com               3    7   377     46   -84us[-81us] +/-  572us
";

    #[test]
    fn parse_chronyc_sources_v_ok() {
        let sources = parse_chronyc_sources_v(CHRONYC_SOURCES_V).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name, "169.254.169.123");
        assert_eq!(sources[0].stratum, Some(2));
        assert_eq!(sources[0].flags.as_deref(), Some("^*"));
        // +15us = 0.000015 seconds
        assert!((sources[0].offset_sec.unwrap() - 0.000015).abs() < 1e-9);
    }

    // --- ntpq -p fixture ---
    const NTPQ_P: &str = "\
     remote           refid      st t when poll reach   delay   offset  jitter
==============================================================================
*ntp1.example.com .GPS.            1 u   16  128  377    1.234   +0.123   0.045
+ntp2.example.com 192.168.1.1      2 u   46  128  377    2.100   -0.084   0.032
-ntp3.example.com 10.0.0.1         3 u  128  128  377    3.000   +0.500   0.100
";

    #[test]
    fn parse_ntpq_p_ok() {
        let s = parse_ntpq_p(NTPQ_P).unwrap();
        assert_eq!(s.sources.len(), 3);
        // Selected source is "ntp1.example.com" (marked with '*').
        assert_eq!(s.reference.as_deref(), Some("ntp1.example.com"));
        assert_eq!(s.stratum, Some(1));
        // Offset: +0.123 ms → 0.000123 s
        assert!((s.offset_sec.unwrap() - 0.000123).abs() < 1e-9);
    }

    #[test]
    fn parse_ntpq_p_no_selected() {
        let input = "\
     remote           refid      st t when poll reach   delay   offset  jitter
==============================================================================
+ntp2.example.com 192.168.1.1      2 u   46  128  377    2.100   -0.084   0.032
";
        let s = parse_ntpq_p(input).unwrap();
        // No '*' line — selected fields are None.
        assert!(s.offset_sec.is_none());
        assert_eq!(s.sources.len(), 1);
    }

    #[test]
    fn chrony_time_offset_microseconds() {
        let v = super::parse_chrony_time_offset("+15us").unwrap();
        assert!((v - 0.000015).abs() < 1e-12);
    }

    #[test]
    fn chrony_time_offset_milliseconds() {
        let v = super::parse_chrony_time_offset("-84ms").unwrap();
        assert!((v - (-0.084)).abs() < 1e-9);
    }

    #[test]
    fn chrony_time_offset_nanoseconds() {
        let v = super::parse_chrony_time_offset("+500ns").unwrap();
        assert!((v - 5e-7).abs() < 1e-12);
    }
}
