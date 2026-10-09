pub mod clock;
pub mod gpsd;
pub mod ntp;
pub mod ssh;

use crate::model::{ServerStatus, ServerUpdate};
use std::time::Duration;
use tokio::sync::mpsc;

/// Minimum backoff duration (1 second).
pub const BACKOFF_MIN: Duration = Duration::from_secs(1);

/// Maximum backoff duration (30 seconds).
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Compute the next exponential backoff duration.
///
/// Starting from `BACKOFF_MIN`, each successive call doubles the previous
/// value up to `BACKOFF_MAX`.
///
/// # Arguments
///
/// * `previous` — the backoff duration used on the previous attempt.
///   Pass `None` (or `Some(Duration::ZERO)`) on the first failure to get
///   `BACKOFF_MIN`.
pub fn next_backoff(previous: Option<Duration>) -> Duration {
    match previous {
        None => BACKOFF_MIN,
        Some(d) if d < BACKOFF_MIN => BACKOFF_MIN,
        Some(d) => {
            let doubled = d.saturating_mul(2);
            doubled.min(BACKOFF_MAX)
        }
    }
}

/// Spawn a per-server ingest task.
///
/// The task runs the main ingest loop: connect via SSH, collect NTP and GPS
/// data, and send `ServerUpdate` messages over `tx`.  On any failure it marks
/// the server `Unreachable`, waits an exponential backoff, and retries.
///
/// The task runs until `tx` is closed (i.e. the receiver in the orchestration
/// loop is dropped), which is the shutdown signal.
///
/// # Arguments
///
/// * `index` — zero-based positional server index.
/// * `hostname` — server hostname or IP address.
/// * `tx` — the mpsc `Sender` to deliver `ServerUpdate` messages on.
pub async fn run_server_ingest(index: usize, hostname: String, tx: mpsc::Sender<ServerUpdate>) {
    let mut backoff: Option<Duration> = None;

    loop {
        // Notify the model that we are attempting a connection.
        if tx
            .send(ServerUpdate::StatusChange {
                index,
                status: ServerStatus::Connecting,
            })
            .await
            .is_err()
        {
            // Receiver dropped — orchestration loop is shutting down.
            return;
        }

        match run_ingest_once(index, &hostname, &tx).await {
            Ok(()) => {
                // Clean exit — sender closed, we are done.
                return;
            }
            Err(e) => {
                // Send the error as a Log message instead of writing to stderr,
                // since the terminal alternate-screen may be active.
                let msg = format!("[trmnlapp] server[{index}] {hostname}: ingest error: {e:#}");
                let _ = tx.send(ServerUpdate::Log(msg)).await;

                // Mark server as unreachable.
                if tx
                    .send(ServerUpdate::StatusChange {
                        index,
                        status: ServerStatus::Unreachable,
                    })
                    .await
                    .is_err()
                {
                    return;
                }

                let delay = next_backoff(backoff);
                backoff = Some(delay);
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Run one ingest session for a single server.
///
/// Returns `Ok(())` when the session should not be retried (e.g. the channel
/// is closed), or `Err(e)` to trigger a reconnect after backoff.
async fn run_ingest_once(
    index: usize,
    hostname: &str,
    tx: &mpsc::Sender<ServerUpdate>,
) -> anyhow::Result<()> {
    use anyhow::Context;

    // Establish SSH session.
    let session = ssh::connect(hostname)
        .await
        .with_context(|| format!("SSH connect to {hostname}"))?;

    // Notify: connection established.
    if tx
        .send(ServerUpdate::StatusChange {
            index,
            status: ServerStatus::Ok,
        })
        .await
        .is_err()
    {
        return Ok(()); // Shutdown.
    }

    // Spawn sub-tasks for NTP and GPS collection.
    // Each uses a clone of the sender.
    let ntp_tx = tx.clone();
    let gps_tx = tx.clone();
    let hostname_ntp = hostname.to_owned();
    let hostname_gps = hostname.to_owned();

    // We need to share the session between tasks.  Use an Arc so both tasks
    // can hold a reference without lifetime issues.
    let session = std::sync::Arc::new(session);
    let session_ntp = std::sync::Arc::clone(&session);
    let session_gps = std::sync::Arc::clone(&session);

    // Poll interval: 1 000 ms (provisional; see contract Q10).
    let poll_interval = Duration::from_millis(1000);

    let ntp_handle = tokio::spawn(async move {
        ntp::run_ntp_loop(index, &hostname_ntp, &session_ntp, poll_interval, ntp_tx).await
    });

    let gps_handle = tokio::spawn(async move {
        gpsd::run_gps_loop(index, &hostname_gps, &session_gps, gps_tx).await
    });

    // Wait for either sub-task to finish.  If one errors, the session is
    // considered failed; the restart loop will reconnect.
    tokio::select! {
        res = ntp_handle => {
            match res {
                Ok(Err(e)) => return Err(e),
                Ok(Ok(())) => {},
                Err(e) => return Err(anyhow::anyhow!("NTP task panicked: {e}")),
            }
        }
        res = gps_handle => {
            match res {
                Ok(Err(e)) => return Err(e),
                Ok(Ok(())) => {},
                Err(e) => return Err(anyhow::anyhow!("GPS task panicked: {e}")),
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_backoff_is_min() {
        assert_eq!(next_backoff(None), BACKOFF_MIN);
    }

    #[test]
    fn backoff_doubles() {
        let b1 = next_backoff(None); // 1s
        let b2 = next_backoff(Some(b1)); // 2s
        let b3 = next_backoff(Some(b2)); // 4s
        assert_eq!(b1, Duration::from_secs(1));
        assert_eq!(b2, Duration::from_secs(2));
        assert_eq!(b3, Duration::from_secs(4));
    }

    #[test]
    fn backoff_caps_at_max() {
        let mut b = BACKOFF_MIN;
        for _ in 0..10 {
            b = next_backoff(Some(b));
        }
        assert_eq!(b, BACKOFF_MAX);
    }

    #[test]
    fn zero_previous_produces_min() {
        assert_eq!(next_backoff(Some(Duration::ZERO)), BACKOFF_MIN);
    }

    #[test]
    fn sub_min_previous_produces_min() {
        assert_eq!(next_backoff(Some(Duration::from_millis(100))), BACKOFF_MIN);
    }
}
