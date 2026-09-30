# stratum

Real-time NTP/GPS timing monitor for chrony+gpsd clusters

stratum is a read-only Rust TUI application that monitors 1–6 LAN NTP/GPS
servers simultaneously over SSH. It displays color-coded offset and frequency
graphs, per-server detail panels, a summary table across all servers, and a
polar satellite sky plot — with no agent or daemon installed on the monitored
servers.

> **Note on first connection:** stratum uses TOFU (Trust On First Use) for SSH
> host-key verification. On first connection to a new server the host key is
> accepted and stored in your `~/.ssh/known_hosts`. This carries the usual TOFU
> MITM risk; verify fingerprints out-of-band if your threat model requires it.

## Features

- Monitors 1–6 servers simultaneously.
- Combined color-coded graphs (one line per server per metric) with axis scales.
- Per-server detail panels.
- Summary table across all servers.
- Polar satellite sky plot.
- SSH-based collection — no agent installed on the monitored servers.
- Supports chrony and `ntpq`.
- gpsd satellite data collected via SSH port-forward.
- Read-only — never configures or mutates devices/servers.
- Runs on Linux x86_64 and macOS arm64.

## Screenshots

*(Add screenshots here — run `trmnlapp pi@ntp1 pi@ntp2 pi@ntp3` for a minute then screenshot)*

## Prerequisites

- Rust stable toolchain.
- SSH access to target servers running chrony/`ntpq` plus gpsd.
- Servers reachable via the user's `~/.ssh/` config (keys, `known_hosts`).

## Installation

```sh
cargo build --release
```

Binary: `target/release/trmnlapp`

## Usage

```
trmnlapp [OPTIONS] <USER@HOST>...
```

Pass 1–6 servers as `user@host` (or bare `host`, which uses `$USER` and your
SSH config).

**Single server:**

```sh
trmnlapp pi@ntp1
```

**Three servers:**

```sh
trmnlapp pi@ntp1 pi@ntp2 pi@ntp3
```

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
