# trmnlapp

A read-only, btop-style terminal UI for monitoring NTP/chrony timing statistics
and gpsd GPS/satellite telemetry on up to three LAN servers over SSH.

## Features

- Collects NTP/chrony statistics (`chronyc tracking`, `chronyc sources -v`; falls
  back to `ntpq -p` when chronyc is absent) via SSH remote commands.
- Collects gpsd telemetry via an SSH local port-forward to the remote gpsd TCP
  port (2947), then speaks the gpsd JSON protocol over that forward.
- In-memory ring-buffer history per server per metric (600 samples, ~10 minutes
  at 1 Hz; no data is persisted across restarts).
- Combined color-coded multi-server graphs (NTP offset, frequency).
- Per-server detail panels (NTP fields, GPS fix/position/satellites).
- Summary table across all servers.
- Stays responsive when a server is unreachable: exponential backoff reconnect,
  per-server status indicator.

## Supported Targets

| Target | Status |
| --- | --- |
| `aarch64-apple-darwin` (macOS arm64) | Primary development target |
| `x86_64-unknown-linux-gnu` | Supported |

Windows and browser/mobile targets are not supported.

## Prerequisites

### Build Requirements

- **Rust stable** (edition 2021; tested with Rust 1.98+).
  Install from <https://rustup.rs>.

### Runtime Requirements

- **System `ssh` client** (`openssh` crate wraps the system binary).
  - macOS: `ssh` ships with the OS.
  - Linux: install `openssh-client` (Debian/Ubuntu) or `openssh-clients` (RHEL/Fedora).
- **`chronyc`** (or `ntpq`) accessible on each monitored server.
- **`gpsd`** running on each monitored server, listening on TCP port 2947
  (default). The user's SSH account must be able to reach `127.0.0.1:2947` on
  each server.
- SSH key-based (passwordless) authentication configured for each server. The
  `openssh` crate uses your `~/.ssh/config` and `~/.ssh/known_hosts` natively.

## Build

```sh
# Development build
cargo build

# Release build (recommended for actual use)
cargo build --release
```

The compiled binary is at `target/release/trmnlapp`.

### Cross-compilation

```sh
# macOS arm64 (on macOS)
cargo build --release --target aarch64-apple-darwin

# Linux x86_64 (requires the target toolchain)
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu
```

## Usage

```sh
trmnlapp <USER@HOST> [USER@HOST2] [USER@HOST3]
```

Pass one to three servers as positional arguments, each in `user@host` or
`host` form.  The order determines the stable color assignment (Cyan, Yellow,
Magenta).

```sh
# Monitor one server (user resolved from SSH config or $USER)
trmnlapp ntp1.lan

# Monitor three servers with explicit usernames
trmnlapp alice@ntp1.lan alice@ntp2.lan alice@ntp3.lan
```

If all servers share the same login, you can omit the username and let your SSH
config supply it via a `User` directive in `~/.ssh/config`:

```
Host ntp*.lan
    User alice
```

Then `trmnlapp ntp1.lan ntp2.lan ntp3.lan` picks up `alice` automatically.

### Keyboard Shortcuts

| Key | Action |
| --- | --- |
| `q` | Quit |
| `Ctrl-C` | Quit |

## Display Layout

```
┌─────────────────────────────────────────────────────────┐
│  Server Summary Table (Host / Status / Offset / ...)    │
├────────────────────────┬────────────────────────────────┤
│  NTP Offset Graph      │  NTP Frequency Graph           │
│  (all servers, color)  │  (all servers, color)          │
├────────┬───────────────┴───────┬────────────────────────┤
│ Server1│ Server2               │ Server3                 │
│ panel  │ panel                 │ panel                   │
└────────┴───────────────────────┴────────────────────────┘
```

All displayed metrics are read-only. The application never modifies servers.

## SSH / Security Notes

This application uses your system `ssh` client and `~/.ssh/` configuration.

### TOFU (Trust On First Use) — Security Disclosure

**When connecting to a server that is not yet in your `~/.ssh/known_hosts`, the
system `ssh` client applies TOFU: it accepts the host's public key and records
it in `known_hosts`.  This automatic first-acceptance means the application
could unknowingly connect to an impersonating host (including a
man-in-the-middle attacker) on a first connection.**

Mitigation steps:
1. Verify the host key fingerprint out-of-band before the first connection.
2. Use `ssh-keyscan -H <hostname>` to pre-populate `known_hosts` after manual
   verification.
3. Alternatively, run `ssh <hostname>` manually first in an interactive terminal
   where you can confirm the fingerprint before using trmnlapp.

**Changed host key:** if a server's host key changes after the initial
connection, trmnlapp treats this as a hard error and will not connect. It will
not silently accept or overwrite the existing `known_hosts` entry.

### Privacy

- Private key material is never copied, stored, or logged.
- No monitoring data is persisted to disk across restarts.
- The only permitted persistent write is recording a genuinely new host key in
  `~/.ssh/known_hosts` via the system `ssh` client.

## Open / Provisional Settings

These defaults are provisional and isolated to single constants for easy change:

| Setting | Default | File |
| --- | --- | --- |
| Ring-buffer capacity | 600 samples | `src/model/ring_buffer.rs` (`CAPACITY`) |
| Refresh interval | 1 000 ms | `src/ingest/mod.rs` (poll loop) |
| Backoff min / max | 1 s / 30 s | `src/ingest/mod.rs` (`BACKOFF_MIN`, `BACKOFF_MAX`) |
| Repaint interval | 250 ms | `src/app.rs` (`REPAINT_INTERVAL`) |
| Server color palette | Cyan / Yellow / Magenta | `src/render/mod.rs` (`server_color`) |

## License

MIT OR Apache-2.0
