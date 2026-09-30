use anyhow::{Context, Result};
use openssh::{ForwardType, KnownHosts, Session, Socket};

/// Establish an SSH `Session` to `hostname` using the user's existing SSH
/// configuration, keys in `~/.ssh/`, and `known_hosts` trust data.
///
/// `KnownHosts::Strict` is used: a genuinely new host is handled by the
/// system `ssh` client (TOFU, may prompt on first connection), but a
/// **changed** host identity is rejected as a connection error — it is never
/// silently accepted.
///
/// Returns an error if the connection fails for any reason (including a
/// changed host key).
pub async fn connect(hostname: &str) -> Result<Session> {
    Session::connect(hostname, KnownHosts::Strict)
        .await
        .with_context(|| format!("SSH connect to '{hostname}' failed"))
}

/// Run a remote command on `session` and return its stdout as a `String`.
///
/// Returns an error if the command exits non-zero or if the output is not
/// valid UTF-8.
pub async fn run_command(session: &Session, program: &str, args: &[&str]) -> Result<String> {
    let mut cmd = session.command(program);
    for arg in args {
        cmd.arg(*arg);
    }
    let output = cmd
        .output()
        .await
        .with_context(|| format!("SSH remote command '{program}' failed"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Remote command '{program}' exited non-zero: {}",
            stderr.trim()
        );
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("Remote command '{program}' produced non-UTF-8 output"))
}

/// Open a local port-forward: connections to `127.0.0.1:local_port` will be
/// forwarded through `session` to `remote_host:remote_port`.
///
/// The forwarding is active as long as `session` is alive.  Call
/// `session.close_port_forward(...)` to tear it down explicitly, or let the
/// session drop.
///
/// The actual openssh API is `Session::request_port_forward(ForwardType,
/// listen_socket, connect_socket)` where the sockets are `Socket` values.
/// Confirmed against openssh-0.11.6 source (`src/session.rs`,
/// `src/port_forwarding.rs`).
pub async fn open_local_forward(
    session: &Session,
    local_port: u16,
    remote_host: &str,
    remote_port: u16,
) -> Result<()> {
    let listen = Socket::new("127.0.0.1", local_port);
    let connect = Socket::new(remote_host, remote_port);

    session
        .request_port_forward(ForwardType::Local, listen, connect)
        .await
        .with_context(|| {
            format!("Port forward 127.0.0.1:{local_port} -> {remote_host}:{remote_port} failed")
        })
}

/// Close a previously opened local port-forward.
pub async fn close_local_forward(
    session: &Session,
    local_port: u16,
    remote_host: &str,
    remote_port: u16,
) -> Result<()> {
    let listen = Socket::new("127.0.0.1", local_port);
    let connect = Socket::new(remote_host, remote_port);

    session
        .close_port_forward(ForwardType::Local, listen, connect)
        .await
        .with_context(|| {
            format!(
                "Close port forward 127.0.0.1:{local_port} -> {remote_host}:{remote_port} failed"
            )
        })
}

// No live-SSH tests in the test suite; parsing logic is in ntp.rs / gpsd.rs
// and tested separately with fixtures (Step 9).
