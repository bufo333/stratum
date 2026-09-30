//! gpsd telemetry collection and parsing.
//!
//! Opens a local SSH port-forward to the remote gpsd TCP port (2947), then
//! speaks the gpsd JSON line protocol (?WATCH/TPV/SKY) over the forwarded
//! port.
//!
//! **Field inventory note (UNVERIFIED against live output):** all struct fields
//! parsed here are `Option<T>`.  Fields that cannot be confirmed from the
//! fixture strings in the test suite must remain as `None`.  See plan
//! section 2 (Q9) for the full list of expected fields.

use std::sync::Arc;

use anyhow::{Context, Result};
use openssh::Session;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::model::{GpsSample, SatelliteInfo, ServerUpdate};

/// Remote gpsd TCP port.
pub const GPSD_PORT: u16 = 2947;

/// The gpsd `?WATCH` enable command (JSON mode + PPS if available).
const WATCH_CMD: &str = "?WATCH={\"enable\":true,\"json\":true}\n";

/// Run the GPS collection loop for one server.
///
/// Opens a local port-forward, connects via TCP, sends `?WATCH`, then reads
/// TPV/SKY messages and forwards them as `ServerUpdate::GpsSample`.
///
/// Returns `Err` to signal the ingest restart loop to reconnect.
/// Returns `Ok(())` when the channel closes (shutdown).
pub async fn run_gps_loop(
    index: usize,
    hostname: &str,
    session: &Arc<Session>,
    tx: mpsc::Sender<ServerUpdate>,
) -> Result<()> {
    // Use an ephemeral high port for the local forward.
    // We pick a fixed per-index port to avoid conflicts among tasks.
    let local_port: u16 = 19200 + index as u16;

    // Open local port-forward: 127.0.0.1:local_port -> remote 127.0.0.1:2947
    super::ssh::open_local_forward(session, local_port, "127.0.0.1", GPSD_PORT)
        .await
        .with_context(|| format!("Port forward for gpsd on {hostname}"))?;

    let result = run_gps_stream(index, hostname, local_port, &tx).await;

    // Attempt to close the port-forward regardless of result.
    let _ = super::ssh::close_local_forward(session, local_port, "127.0.0.1", GPSD_PORT).await;

    result
}

/// Connect to the forwarded gpsd port and read messages until an error or
/// channel close.
async fn run_gps_stream(
    index: usize,
    hostname: &str,
    local_port: u16,
    tx: &mpsc::Sender<ServerUpdate>,
) -> Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port))
        .await
        .with_context(|| format!("TCP connect to forwarded gpsd port {local_port} ({hostname})"))?;

    stream
        .write_all(WATCH_CMD.as_bytes())
        .await
        .context("Write ?WATCH to gpsd")?;

    let mut latest_tpv: Option<TpvMessage> = None;
    let mut latest_sky: Option<SkyMessage> = None;

    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => {
                anyhow::bail!("gpsd connection closed by remote ({hostname})");
            }
            Err(e) => {
                return Err(e).with_context(|| format!("gpsd read error ({hostname})"));
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Ok(Some(msg)) = parse_gpsd_line(line) {
            match msg {
                GpsdMessage::Tpv(tpv) => {
                    latest_tpv = Some(tpv);
                }
                GpsdMessage::Sky(sky) => {
                    latest_sky = Some(sky);
                }
                GpsdMessage::Other => {}
            }
        }

        // Whenever we have a TPV, emit a sample (GPS + satellites if available).
        if let Some(ref tpv) = latest_tpv {
            let satellites = latest_sky
                .as_ref()
                .map(sky_to_satellites)
                .unwrap_or_default();

            let sample = GpsSample {
                fix_mode: tpv.mode,
                lat: tpv.lat,
                lon: tpv.lon,
                alt_m: tpv.alt,
                time: tpv.time.clone(),
                ept: tpv.ept,
                satellites,
            };

            if tx
                .send(ServerUpdate::GpsSample { index, sample })
                .await
                .is_err()
            {
                return Ok(()); // Channel closed — shutdown.
            }

            // Clear TPV so we don't re-emit the same fix every loop iteration.
            latest_tpv = None;
        }
    }
}

// ---------------------------------------------------------------------------
// gpsd JSON message types
// ---------------------------------------------------------------------------

/// A discriminated union of the gpsd JSON message classes we handle.
#[derive(Debug)]
pub(crate) enum GpsdMessage {
    Tpv(TpvMessage),
    Sky(SkyMessage),
    Other,
}

/// gpsd `TPV` (time-position-velocity) class.
///
/// Fields confirmed present in gpsd documentation and commonly available.
/// All are `Option` because a partial fix may omit some.
///
/// UNVERIFIED against live gpsd output (plan Q9).
#[derive(Debug, Deserialize)]
pub(crate) struct TpvMessage {
    /// Fix mode: 0 = unknown, 1 = no fix, 2 = 2D, 3 = 3D.
    mode: Option<u8>,
    /// Latitude (degrees, WGS-84).
    lat: Option<f64>,
    /// Longitude (degrees, WGS-84).
    lon: Option<f64>,
    /// Altitude above mean sea level (metres).
    alt: Option<f64>,
    /// UTC time string (ISO 8601).
    time: Option<String>,
    /// Estimated time error (seconds, 95% confidence).
    ept: Option<f64>,
}

/// gpsd `SKY` class.
///
/// UNVERIFIED against live gpsd output (plan Q9).
#[derive(Debug, Deserialize)]
pub(crate) struct SkyMessage {
    /// Array of satellites.
    satellites: Option<Vec<SkyEntry>>,
}

/// One satellite entry in the `SKY` class.
///
/// UNVERIFIED against live gpsd output (plan Q9).
#[derive(Debug, Deserialize)]
struct SkyEntry {
    /// PRN / satellite ID.
    #[serde(rename = "PRN")]
    prn: Option<i32>,
    /// Whether used in fix.
    used: Option<bool>,
    /// Elevation (degrees).
    el: Option<f64>,
    /// Azimuth (degrees).
    az: Option<f64>,
    /// Signal-to-noise (dB-Hz).
    ss: Option<f64>,
    /// GNSS system ID (0=GPS, 1=SBAS, 2=Galileo, 3=BeiDou, 4=IMES, 5=QZSS, 6=GLONASS).
    gnssid: Option<u8>,
}

/// Convert `SkyEntry` slice to `Vec<SatelliteInfo>`.
fn sky_to_satellites(sky: &SkyMessage) -> Vec<SatelliteInfo> {
    sky.satellites
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|e| SatelliteInfo {
            prn: e.prn,
            used: e.used,
            elevation: e.el,
            azimuth: e.az,
            snr: e.ss,
            gnss_id: e.gnssid.map(gnssid_to_string),
        })
        .collect()
}

/// Map gpsd integer `gnssid` to a human-readable constellation name.
///
/// Based on gpsd documentation (UNVERIFIED against live output).
fn gnssid_to_string(id: u8) -> String {
    match id {
        0 => "GPS",
        1 => "SBAS",
        2 => "Galileo",
        3 => "BeiDou",
        4 => "IMES",
        5 => "QZSS",
        6 => "GLONASS",
        _ => "Unknown",
    }
    .to_owned()
}

/// Parse a single gpsd JSON line.
///
/// Returns `Ok(Some(msg))` on a recognized message, `Ok(None)` for an
/// unrecognized-but-valid JSON class, `Err` for malformed JSON.
pub fn parse_gpsd_line(line: &str) -> Result<Option<GpsdMessage>> {
    // Quick check for the "class" field before full deserialization.
    let raw: serde_json::Value =
        serde_json::from_str(line).with_context(|| "gpsd JSON parse error")?;

    let class = match raw.get("class").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return Ok(Some(GpsdMessage::Other)),
    };

    match class {
        "TPV" => {
            let tpv: TpvMessage =
                serde_json::from_value(raw).context("gpsd TPV deserialization")?;
            Ok(Some(GpsdMessage::Tpv(tpv)))
        }
        "SKY" => {
            let sky: SkyMessage =
                serde_json::from_value(raw).context("gpsd SKY deserialization")?;
            Ok(Some(GpsdMessage::Sky(sky)))
        }
        _ => Ok(Some(GpsdMessage::Other)),
    }
}

// ---------------------------------------------------------------------------
// Tests (Step 9 fixtures)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Representative gpsd JSON fixtures (UNVERIFIED against live output).

    const TPV_3D_FIX: &str = r#"{"class":"TPV","device":"/dev/ttyS0","mode":3,"time":"2024-01-01T00:00:00.000Z","ept":0.005,"lat":37.7749,"lon":-122.4194,"alt":50.0,"epx":5.1,"epy":6.2}"#;

    const TPV_NO_FIX: &str = r#"{"class":"TPV","mode":1}"#;

    const SKY_WITH_SATS: &str = r#"{"class":"SKY","satellites":[{"PRN":1,"used":true,"el":45.0,"az":180.0,"ss":38.0,"gnssid":0},{"PRN":20,"used":false,"el":12.0,"az":90.0,"ss":22.0,"gnssid":0},{"PRN":65,"used":true,"el":30.0,"az":270.0,"ss":35.0,"gnssid":6}]}"#;

    const WATCH_RESPONSE: &str = r#"{"class":"WATCH","enable":true,"json":true}"#;

    const DEVICES_MSG: &str = r#"{"class":"DEVICES","devices":[]}"#;

    #[test]
    fn parse_tpv_3d_fix() {
        let msg = parse_gpsd_line(TPV_3D_FIX).unwrap().unwrap();
        if let GpsdMessage::Tpv(tpv) = msg {
            assert_eq!(tpv.mode, Some(3));
            assert!((tpv.lat.unwrap() - 37.7749).abs() < 1e-4);
            assert!((tpv.lon.unwrap() - (-122.4194)).abs() < 1e-4);
            assert!((tpv.alt.unwrap() - 50.0).abs() < 0.01);
            assert_eq!(tpv.time.as_deref(), Some("2024-01-01T00:00:00.000Z"));
        } else {
            panic!("expected TPV");
        }
    }

    #[test]
    fn parse_tpv_no_fix() {
        let msg = parse_gpsd_line(TPV_NO_FIX).unwrap().unwrap();
        if let GpsdMessage::Tpv(tpv) = msg {
            assert_eq!(tpv.mode, Some(1));
            assert!(tpv.lat.is_none());
            assert!(tpv.lon.is_none());
        } else {
            panic!("expected TPV");
        }
    }

    #[test]
    fn parse_sky_with_satellites() {
        let msg = parse_gpsd_line(SKY_WITH_SATS).unwrap().unwrap();
        if let GpsdMessage::Sky(sky) = msg {
            let sats = sky_to_satellites(&sky);
            assert_eq!(sats.len(), 3);
            // First satellite: GPS PRN 1, used
            assert_eq!(sats[0].prn, Some(1));
            assert_eq!(sats[0].used, Some(true));
            assert_eq!(sats[0].gnss_id.as_deref(), Some("GPS"));
            // Third satellite: GLONASS
            assert_eq!(sats[2].prn, Some(65));
            assert_eq!(sats[2].used, Some(true));
            assert_eq!(sats[2].gnss_id.as_deref(), Some("GLONASS"));
        } else {
            panic!("expected SKY");
        }
    }

    #[test]
    fn parse_watch_response_is_other() {
        let msg = parse_gpsd_line(WATCH_RESPONSE).unwrap().unwrap();
        assert!(matches!(msg, GpsdMessage::Other));
    }

    #[test]
    fn parse_devices_is_other() {
        let msg = parse_gpsd_line(DEVICES_MSG).unwrap().unwrap();
        assert!(matches!(msg, GpsdMessage::Other));
    }

    #[test]
    fn parse_malformed_json_errors() {
        assert!(parse_gpsd_line("{not valid json}").is_err());
    }

    #[test]
    fn gnssid_mapping() {
        assert_eq!(gnssid_to_string(0), "GPS");
        assert_eq!(gnssid_to_string(6), "GLONASS");
        assert_eq!(gnssid_to_string(2), "Galileo");
        assert_eq!(gnssid_to_string(99), "Unknown");
    }
}
