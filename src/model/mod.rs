pub mod ring_buffer;
pub mod server;

pub use server::{
    GpsSample, NtpSample, NtpSource, SatelliteInfo, ServerState, ServerStatus, ServerUpdate,
};

/// Application-wide model.
///
/// Owned exclusively by the orchestration loop in `src/app.rs`.
/// No I/O, no network, no subprocess calls.
#[derive(Debug)]
pub struct Model {
    servers: Vec<ServerState>,
}

impl Model {
    /// Construct a model for the given server hostnames (1–3).
    pub fn new(hostnames: Vec<String>) -> Self {
        let servers = hostnames
            .into_iter()
            .enumerate()
            .map(|(idx, host)| ServerState::new(host, idx))
            .collect();
        Self { servers }
    }

    /// Apply a `ServerUpdate` message produced by an ingest task.
    pub fn apply(&mut self, update: ServerUpdate) {
        match update {
            ServerUpdate::NtpSample { index, sample } => {
                if let Some(s) = self.servers.get_mut(index) {
                    s.apply_ntp(sample);
                }
            }
            ServerUpdate::GpsSample { index, sample } => {
                if let Some(s) = self.servers.get_mut(index) {
                    s.apply_gps(sample);
                }
            }
            ServerUpdate::StatusChange { index, status } => {
                if let Some(s) = self.servers.get_mut(index) {
                    s.set_status(status);
                }
            }
            // Diagnostic log from an ingest task; discarded for now.
            ServerUpdate::Log(_) => {}
        }
    }

    /// Read-only slice of all server states.
    pub fn servers(&self) -> &[ServerState] {
        &self.servers
    }

    /// Read-only reference to a single server state by index.
    pub fn server(&self, index: usize) -> Option<&ServerState> {
        self.servers.get(index)
    }

    /// Number of servers being monitored.
    pub fn server_count(&self) -> usize {
        self.servers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use server::{NtpSample, ServerStatus};

    fn make_model(n: usize) -> Model {
        let hosts = (0..n).map(|i| format!("server{i}")).collect();
        Model::new(hosts)
    }

    #[test]
    fn initial_status_is_connecting() {
        let m = make_model(2);
        for s in m.servers() {
            assert_eq!(s.status(), ServerStatus::Connecting);
        }
    }

    #[test]
    fn status_change_unreachable_to_ok() {
        let mut m = make_model(1);
        m.apply(ServerUpdate::StatusChange {
            index: 0,
            status: ServerStatus::Unreachable,
        });
        assert_eq!(m.server(0).unwrap().status(), ServerStatus::Unreachable);

        m.apply(ServerUpdate::StatusChange {
            index: 0,
            status: ServerStatus::Ok,
        });
        assert_eq!(m.server(0).unwrap().status(), ServerStatus::Ok);
    }

    #[test]
    fn ntp_sample_pushed_to_history() {
        let mut m = make_model(1);
        m.apply(ServerUpdate::StatusChange {
            index: 0,
            status: ServerStatus::Ok,
        });
        let sample = NtpSample {
            offset_sec: Some(0.000123),
            rms_offset_sec: Some(0.000045),
            freq_ppm: Some(-5.2),
            stratum: Some(2),
            reference: Some("GPS".into()),
            root_delay_sec: Some(0.001),
            root_dispersion_sec: Some(0.002),
            leap_status: Some("Normal".into()),
            sources: vec![],
        };
        m.apply(ServerUpdate::NtpSample { index: 0, sample });
        let server = m.server(0).unwrap();
        assert_eq!(server.ntp_history().len(), 1);
    }

    #[test]
    fn out_of_range_index_is_ignored() {
        let mut m = make_model(1);
        // Index 5 does not exist; should not panic.
        m.apply(ServerUpdate::StatusChange {
            index: 5,
            status: ServerStatus::Ok,
        });
    }
}
