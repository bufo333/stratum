use std::time::{Duration, Instant};

use super::ring_buffer::RingBuffer;

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Connection / data-freshness status of a monitored server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    /// Initial state: ingest task is starting / establishing the SSH session.
    Connecting,
    /// The SSH session is up and data is flowing.
    Ok,
    /// Data was received but has not been refreshed for longer than the
    /// stale threshold.
    #[allow(dead_code)]
    Stale,
    /// The server cannot be reached (SSH failure, timeout, etc.).
    Unreachable,
}

// ---------------------------------------------------------------------------
// NTP / chrony sample
// ---------------------------------------------------------------------------

/// Receipt freshness for the Timing view only.
pub const TIMING_FRESHNESS: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NtpProvenance {
    ChronyTracking,
    NtpqPeer,
}

/// Host system clock captured over SSH, associated with a local midpoint.
#[derive(Debug, Clone)]
pub struct ClockSample {
    pub epoch_ns: u64,
    pub midpoint: Instant,
    pub received_at: Instant,
    pub rtt: Duration,
}

impl ClockSample {
    pub fn projected_ns(&self, now: Instant) -> Option<u64> {
        let elapsed = u64::try_from(now.checked_duration_since(self.midpoint)?.as_nanos()).ok()?;
        self.epoch_ns.checked_add(elapsed)
    }
}

pub fn sample_age(received_at: Option<Instant>, now: Instant) -> Option<Duration> {
    now.checked_duration_since(received_at?)
}

pub fn is_fresh(status: ServerStatus, received_at: Option<Instant>, now: Instant) -> bool {
    status == ServerStatus::Ok
        && sample_age(received_at, now).is_some_and(|age| age <= TIMING_FRESHNESS)
}

/// One poll result from chrony (or ntpq fallback).
///
/// Fields marked `Option` were not confirmed from static fixture data at plan
/// time; they render as "n/a" when `None`.
#[derive(Debug, Clone)]
pub struct NtpSample {
    pub provenance: NtpProvenance,
    /// Chrony Last offset or ntpq selected-peer offset (seconds); see provenance.
    pub offset_sec: Option<f64>,
    /// RMS offset (seconds).
    pub rms_offset_sec: Option<f64>,
    /// Clock frequency error (ppm).
    pub freq_ppm: Option<f64>,
    /// NTP stratum of this host.
    pub stratum: Option<u8>,
    /// Reference source name / IP.
    pub reference: Option<String>,
    /// Root delay (seconds).
    pub root_delay_sec: Option<f64>,
    /// Root dispersion (seconds).
    pub root_dispersion_sec: Option<f64>,
    /// Leap-second indicator text ("Normal", "InsertSecond", etc.).
    pub leap_status: Option<String>,
    /// Per-source rows from `chronyc sources -v` or `ntpq -p`.
    #[allow(dead_code)]
    pub sources: Vec<NtpSource>,
}

/// One row from `chronyc sources -v` or `ntpq -p`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NtpSource {
    /// Source name or IP address.
    pub name: String,
    /// Stratum of the source.
    pub stratum: Option<u8>,
    /// Poll interval exponent.
    pub poll: Option<i32>,
    /// Reachability register (octal bitmask of last 8 polls).
    pub reach: Option<u8>,
    /// Seconds since last receive.
    pub last_rx_sec: Option<i64>,
    /// Clock offset (seconds).
    pub offset_sec: Option<f64>,
    /// Mode/state flags as a string (e.g. "^*" for chrony, "*" for ntpq).
    pub flags: Option<String>,
}

// ---------------------------------------------------------------------------
// GPS / gpsd sample
// ---------------------------------------------------------------------------

/// One snapshot from the gpsd `TPV` + `SKY` classes.
///
/// Fields are `Option` because fix availability varies and the field inventory
/// is UNVERIFIED against live output (plan section 2, Q9).
#[derive(Debug, Clone)]
pub struct GpsSample {
    /// Fix mode: 0 = unknown, 1 = no fix, 2 = 2D, 3 = 3D.
    pub fix_mode: Option<u8>,
    /// Latitude (degrees, WGS-84).
    pub lat: Option<f64>,
    /// Longitude (degrees, WGS-84).
    pub lon: Option<f64>,
    /// Altitude above mean sea level (metres).
    pub alt_m: Option<f64>,
    /// UTC time string from the receiver.
    #[allow(dead_code)]
    pub time: Option<String>,
    /// Estimated position error (metres).
    #[allow(dead_code)]
    pub ept: Option<f64>,
    /// Satellites visible in the SKY message.
    pub satellites: Vec<SatelliteInfo>,
}

/// One satellite entry from the gpsd `SKY` class.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SatelliteInfo {
    /// Satellite PRN/ID.
    pub prn: Option<i32>,
    /// Whether this satellite is used in the fix.
    pub used: Option<bool>,
    /// Elevation angle (degrees).
    pub elevation: Option<f64>,
    /// Azimuth (degrees).
    pub azimuth: Option<f64>,
    /// Signal-to-noise ratio (dB-Hz).
    pub snr: Option<f64>,
    /// GNSS constellation ID where available (e.g. "GPS", "GLONASS", "Galileo").
    pub gnss_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Update messages sent from ingest tasks to the orchestration loop
// ---------------------------------------------------------------------------

/// Message type produced by ingest tasks and consumed by the orchestration
/// loop.  Ingest tasks never mutate the `Model` directly.
#[derive(Debug)]
pub enum ServerUpdate {
    /// A fresh NTP/chrony poll result.
    NtpSample {
        /// Zero-based server index (positional CLI argument order).
        index: usize,
        sample: NtpSample,
        received_at: Instant,
    },
    /// Optional capture; a failure immediately replaces availability.
    ClockResult {
        index: usize,
        sample: Option<ClockSample>,
    },
    /// A fresh GPS snapshot from gpsd.
    GpsSample {
        /// Zero-based server index.
        index: usize,
        sample: GpsSample,
    },
    /// A change in the server's connection / data status.
    StatusChange {
        /// Zero-based server index.
        index: usize,
        status: ServerStatus,
    },
    /// A diagnostic log message from an ingest task.
    ///
    /// Used instead of `eprintln!` so that ingest tasks do not write to
    /// stderr while the terminal alternate-screen is active.
    #[allow(dead_code)]
    Log(String),
}

// ---------------------------------------------------------------------------
// Per-server state
// ---------------------------------------------------------------------------

/// All state held for one monitored server.
#[derive(Debug)]
pub struct ServerState {
    /// Hostname / IP as provided on the command line.
    pub hostname: String,
    /// Zero-based positional index (used to assign a stable display color).
    pub index: usize,
    status: ServerStatus,
    /// Historical NTP samples (ring buffer, newest at back).
    ntp_history: RingBuffer<NtpSample>,
    /// Historical GPS samples.
    gps_history: RingBuffer<GpsSample>,
    /// Most recently received NTP sample (also the last entry in `ntp_history`).
    latest_ntp: Option<NtpSample>,
    ntp_received_at: Option<Instant>,
    clock: Option<ClockSample>,
    /// Most recently received GPS sample.
    latest_gps: Option<GpsSample>,
}

impl ServerState {
    /// Construct a new `ServerState` in the `Connecting` status.
    pub fn new(hostname: String, index: usize) -> Self {
        Self {
            hostname,
            index,
            status: ServerStatus::Connecting,
            ntp_history: RingBuffer::new(),
            gps_history: RingBuffer::new(),
            latest_ntp: None,
            ntp_received_at: None,
            clock: None,
            latest_gps: None,
        }
    }

    // -----------------------------------------------------------------------
    // Transition helpers (called by Model::apply)
    // -----------------------------------------------------------------------

    pub(super) fn apply_ntp(&mut self, sample: NtpSample, received_at: Instant) {
        self.ntp_received_at = Some(received_at);
        self.latest_ntp = Some(sample.clone());
        self.ntp_history.push(sample);
        if self.status == ServerStatus::Connecting || self.status == ServerStatus::Unreachable {
            self.status = ServerStatus::Ok;
        }
    }

    pub(super) fn apply_gps(&mut self, sample: GpsSample) {
        self.latest_gps = Some(sample.clone());
        self.gps_history.push(sample);
    }

    pub(super) fn set_status(&mut self, status: ServerStatus) {
        self.status = status;
    }

    // -----------------------------------------------------------------------
    // Read-only accessors
    // -----------------------------------------------------------------------

    pub(super) fn apply_clock(&mut self, sample: Option<ClockSample>) {
        self.clock = sample;
    }

    pub fn clock(&self) -> Option<&ClockSample> {
        self.clock.as_ref()
    }

    pub fn ntp_received_at(&self) -> Option<Instant> {
        self.ntp_received_at
    }

    pub fn timing_offset(&self, now: Instant) -> Option<f64> {
        if !is_fresh(self.status, self.ntp_received_at, now) {
            return None;
        }
        let sample = self.latest_ntp()?;
        if sample.provenance != NtpProvenance::ChronyTracking {
            return None;
        }
        sample.offset_sec.filter(|value| value.is_finite())
    }

    pub fn status(&self) -> ServerStatus {
        self.status
    }

    pub fn latest_ntp(&self) -> Option<&NtpSample> {
        self.latest_ntp.as_ref()
    }

    pub fn latest_gps(&self) -> Option<&GpsSample> {
        self.latest_gps.as_ref()
    }

    pub fn ntp_history(&self) -> &RingBuffer<NtpSample> {
        &self.ntp_history
    }

    #[allow(dead_code)]
    pub fn gps_history(&self) -> &RingBuffer<GpsSample> {
        &self.gps_history
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ntp() -> NtpSample {
        NtpSample {
            provenance: NtpProvenance::ChronyTracking,
            offset_sec: Some(0.0001),
            rms_offset_sec: Some(0.00005),
            freq_ppm: Some(-3.1),
            stratum: Some(2),
            reference: Some("GPS".into()),
            root_delay_sec: Some(0.001),
            root_dispersion_sec: Some(0.002),
            leap_status: Some("Normal".into()),
            sources: vec![],
        }
    }

    fn sample_gps() -> GpsSample {
        GpsSample {
            fix_mode: Some(3),
            lat: Some(37.7749),
            lon: Some(-122.4194),
            alt_m: Some(50.0),
            time: Some("2024-01-01T00:00:00.000Z".into()),
            ept: Some(0.5),
            satellites: vec![],
        }
    }

    #[test]
    fn initial_status_is_connecting() {
        let s = ServerState::new("host1".into(), 0);
        assert_eq!(s.status(), ServerStatus::Connecting);
        assert!(s.latest_ntp().is_none());
        assert!(s.latest_gps().is_none());
        assert!(s.ntp_history().is_empty());
    }

    #[test]
    fn apply_ntp_transitions_to_ok() {
        let mut s = ServerState::new("host1".into(), 0);
        s.apply_ntp(sample_ntp(), Instant::now());
        assert_eq!(s.status(), ServerStatus::Ok);
        assert!(s.latest_ntp().is_some());
        assert_eq!(s.ntp_history().len(), 1);
    }

    #[test]
    fn apply_ntp_after_unreachable_recovers_to_ok() {
        let mut s = ServerState::new("host1".into(), 0);
        s.set_status(ServerStatus::Unreachable);
        s.apply_ntp(sample_ntp(), Instant::now());
        assert_eq!(s.status(), ServerStatus::Ok);
    }

    #[test]
    fn gps_sample_stored() {
        let mut s = ServerState::new("host1".into(), 0);
        s.apply_gps(sample_gps());
        assert!(s.latest_gps().is_some());
        assert_eq!(s.gps_history().len(), 1);
    }

    #[test]
    fn ntp_history_grows() {
        let mut s = ServerState::new("h".into(), 0);
        for _ in 0..5 {
            s.apply_ntp(sample_ntp(), Instant::now());
        }
        assert_eq!(s.ntp_history().len(), 5);
    }

    #[test]
    fn stale_status_does_not_auto_clear_on_sample() {
        // A fresh sample always moves status to Ok regardless of Stale.
        let mut s = ServerState::new("h".into(), 0);
        s.set_status(ServerStatus::Stale);
        // apply_ntp only promotes from Connecting/Unreachable; Stale → Ok is
        // handled by the orchestration layer's stale detection, so we verify
        // it does NOT unexpectedly reset here.
        // The rule: apply_ntp transitions Connecting|Unreachable → Ok.
        assert_eq!(s.status(), ServerStatus::Stale);
        s.apply_ntp(sample_ntp(), Instant::now());
        // Stale is NOT Connecting or Unreachable so status stays Stale.
        assert_eq!(s.status(), ServerStatus::Stale);
    }
    #[test]
    fn independent_freshness_failure_and_recovery() {
        let now = Instant::now();
        let mut server = ServerState::new("synthetic".into(), 0);
        server.apply_ntp(sample_ntp(), now);
        server.apply_clock(Some(ClockSample {
            epoch_ns: 0,
            midpoint: now,
            received_at: now,
            rtt: Duration::ZERO,
        }));
        let boundary = now + TIMING_FRESHNESS;
        assert!(server.timing_offset(boundary).is_some());
        assert!(server
            .timing_offset(boundary + Duration::from_nanos(1))
            .is_none());
        server.apply_gps(sample_gps());
        assert!(server
            .timing_offset(boundary + Duration::from_nanos(1))
            .is_none());
        server.apply_clock(None);
        assert!(server.clock().is_none());
        assert!(server.timing_offset(now).is_some());
        server.apply_clock(Some(ClockSample {
            epoch_ns: 0,
            midpoint: boundary,
            received_at: boundary,
            rtt: Duration::ZERO,
        }));
        assert!(is_fresh(
            server.status(),
            server.clock().map(|s| s.received_at),
            boundary + Duration::from_secs(1)
        ));
        assert!(server
            .timing_offset(boundary + Duration::from_secs(1))
            .is_none());
        for status in [
            ServerStatus::Connecting,
            ServerStatus::Unreachable,
            ServerStatus::Stale,
        ] {
            server.set_status(status);
            assert!(server.timing_offset(now).is_none());
            assert!(!is_fresh(status, Some(now), now));
        }
        server.set_status(ServerStatus::Unreachable);
        server.apply_ntp(sample_ntp(), boundary);
        assert!(server.timing_offset(boundary).is_some());
        let mut ntpq = sample_ntp();
        ntpq.provenance = NtpProvenance::NtpqPeer;
        server.apply_ntp(ntpq, now);
        assert!(server.timing_offset(now).is_none());
        for value in [None, Some(f64::NAN), Some(f64::INFINITY)] {
            let mut sample = sample_ntp();
            sample.offset_sec = value;
            server.apply_ntp(sample, now);
            assert!(server.timing_offset(now).is_none());
        }
    }
}
