# Engineering Contract — trmnlapp

## 1. Problem and Scope

trmnlapp is a read-only TUI monitoring application in the style of btop. It
monitors up to six LAN servers, each running chrony and gpsd. The user
specifies servers as CLI positional arguments.

**Confirmed in scope:**
- Read-only diagnostics only. No device configuration, no server mutations.
- Display NTP/chrony timing statistics (offset, jitter, drift at minimum) and
  gpsd satellite information, GPS lock/fix status.
- Constellation identification (GPS, GLONASS, etc.) where telemetry supports it.
  Do not invent unavailable fields.
- Combined color-coded graphs with one line per server per metric, per-server
  panels, and a summary table.
- Collection over SSH (preferred transport).
- Runs on Linux and macOS.
- Approved integrated timing boxes: optional SSH host-system UTC estimates,
  chrony Last offset instruments and light-distance equivalents alongside the
  existing dashboard.

**Confirmed out of scope:**
- TimeClick serial/USB support (potential future extension; not current scope).
- Server or device configuration changes.
- Persistent monitoring data across restarts (in-memory ring buffers only; see
  SSH exception in Section 7).

**Open question:** Optional horizon/sky-view satellite display — product decision
pending. Do not implement without explicit approval.

**Open question:** Exact metric field names, units, and telemetry mappings must
be verified during planning against actual gpsd JSON and chronyc output. Do not
assert them in governance or planning without source evidence.

## 2. Language, Toolchain, and Dependencies

| Item | Decision |
| --- | --- |
| Language | Rust |
| Channel | Latest stable (no nightly) |
| Edition | 2021 |
| Package manager | Cargo |
| TUI framework | ratatui + crossterm backend |
| Async runtime | tokio, multi-thread scheduler |

**Quality gate (required — all three must pass):**

```sh
cargo fmt --check && cargo test && cargo build --release
```

`cargo clippy -- -D warnings` is recommended but not yet enforced as a hard
gate. This gate applies to all application delivery; governance-only baseline
commits are excluded.

**Open questions:** Dependency admission policy (preferred licenses, prohibited
licenses), exact crate versions, whether to pin a toolchain file.

## 3. Supported Targets

| Target | Status |
| --- | --- |
| Linux (primary) | Required. Primary target: `x86_64-unknown-linux-gnu`. |
| macOS (development + end-user) | Required. |

**Open questions:** Exact macOS architecture (arm64 / x86_64 / universal) and
minimum macOS version. Minimum Linux kernel/glibc version. Windows and
browser/mobile targets are not in scope; this is an unresolved absence, not an
explicit exclusion policy.

## 4. Architecture

The application has three data layers with an explicit orchestration concern:

- **Ingest** — read-only collection I/O, SSH authentication integration, and
  telemetry parsing. Sends normalized updates; does not mutate shared state
  directly, import widget modules, or invoke rendering.
- **Model** — owns per-server current state and in-memory history. No network,
  subprocess, file I/O, or rendering. Does not depend on ingest or widget
  implementation modules.
- **Widgets** — view composition only. Reads model snapshots; produces ratatui
  layout/widget trees. No terminal I/O, no event handling, no filesystem or
  network I/O, no model mutation.
- **Orchestration** (`main.rs` or `src/app.rs`) — CLI argument parsing, tokio
  runtime setup, ingest task lifecycle, update-consumer loop, terminal
  setup/restoration, crossterm event handling, draw scheduling, and
  shutdown/cleanup. Drives drawing using widgets and model snapshots. Terminal
  I/O belongs here, not in widget modules.

These boundaries are confirmed architectural constraints, not proposals.
Concrete module names, channel/locking APIs, and data structure choices are
planning decisions to be resolved in an approved plan.

**Open question:** Whether orchestration lives entirely in `main.rs` or is split
into a dedicated `src/app.rs` module.

## 5. Correctness and Reliability

- Servers are identified by their positional CLI argument order. Up to six
  servers accepted; zero or more than six is a usage error.
- Servers receive stable distinct colors in the display. Exact palette and
  accessibility/color policy are open questions.
- In-memory per-server history via fixed-capacity ring buffers. No persistence
  of monitoring data across restarts (SSH/known_hosts exception applies; see
  Section 7).
- TUI remains responsive while servers are unreachable.

**Open questions:** Maximum ring buffer size / history retention duration.
Refresh frequency. Reconnect/backoff behavior. Stale-data handling and error
presentation. Measurable performance/reliability expectations.

## 6. SSH, Privacy, and Security

The application collects diagnostics from the LAN servers over SSH
(preferred transport).

**Rules:**
- Use the user's SSH configuration, keys in `~/.ssh/`, and `known_hosts` trust
  data.
- Do not copy, store, or log private-key material or passwords.
- New-host auto-acceptance is TOFU (trust on first use): a genuinely unknown
  identity may be accepted and recorded in `known_hosts`.
- A **changed** known-host identity must NOT be silently accepted or overwritten.
  Treat a changed identity as an error.
- **First-connection TOFU risk:** automatic trust on first use can accept an
  impersonating host, including a man-in-the-middle. This risk is inherent in
  TOFU and must be disclosed in user-facing documentation.
- Recording a genuinely new host in `known_hosts` is a permitted exception to
  the in-memory-only monitoring storage policy. It does not authorize persistent
  telemetry, credential copies, logs, or remote mutation.
- SSH collection is for read-only diagnostics only; no privileged changes.

**Open questions:** SSH library vs. executable selection, connection management
details (session reuse, per-server parallelism, reconnect behavior), remote
command selection and invocation.

## 7. External Services and Data Sources

- All LAN servers run chrony and gpsd.
- Collection is over SSH (preferred). No protocol field, endpoint path, command
  availability, exact version, or schema is asserted as verified until confirmed
  against actual server output during planning.
- NTP/chrony exact query mechanism (subprocess `chronyc`/`ntpq` vs. direct
  protocol vs. SSH remote command) is an open question.
- gpsd telemetry: connection mechanism is an open question. Exact field
  inventory, units, and semantics must be verified from gpsd documentation or
  actual output during planning.

**Open question:** PPS device support — future extension point, not current
scope.

## 8. Tests

- Unit tests for ring-buffer correctness and state transitions are expected.
- Integration tests for ingest parsing with mocked output are expected.
- Test fixtures must not contain private-key material or passwords.

**Open questions:** Coverage thresholds, test platform matrix (Linux and macOS),
SSH trust/error-path test cases, CI configuration and hosting, manual check
requirements.

## 9. Documentation and Delivery

- A user-facing installation and usage README is expected.
- TOFU first-connection risk must be documented for end users.

**Open questions:** Changelog format, versioning scheme, migration policy,
release process, additional Git conventions beyond the scaffold rules.

## 10. Git and Workflow

Scaffold rules (from `docs/agent-workflow.md`) apply:

- `main` is the local integration branch.
- Application delivery uses a temporary approved implementation branch; at most
  one in flight; plan is immutable once approved; review targets the exact
  committed revision; integration is fast-forward followed by branch deletion.
- **One branch at a time:** Only one implementation branch may be open
  concurrently. A `branch-bootstrap` agent must not create a new branch until
  the previous feature branch has been integrated (fast-forwarded) into `main`.
  Stacking unmerged branches is prohibited.
- Explicit Git permission prompts are required.
- All remote operations are user-owned. Agents never push, fetch, pull, alter
  remotes, or use GitHub.

The governance baseline commit uses the bootstrap procedure without creating a
branch.

## 11. Open Questions Register

Items below require user resolution before any dependent implementation begins.
Do not invent policy for them.

| # | Open question | Who resolves |
| --- | --- | --- |
| 1 | PPS device support — future extension point | User via coordinator |
| 2 | TimeClick serial/USB — potential future extension | User via coordinator |
| 3 | Exact NTP/chrony query mechanism (subprocess vs. direct protocol vs. SSH remote command) | User via coordinator; planner verifies |
| 4 | Server color palette and accessibility/color policy | **Palette resolved** — six-server palette: 0 Cyan, 1 Yellow, 2 Magenta, 3 Green, 4 Red, 5 Blue, wrapping at 6 (`index % 6`). Broader accessibility/color-blind policy remains open. |
| 5 | Maximum ring buffer size / history retention duration | User via coordinator |
| 6 | Exact macOS architecture and minimum macOS version | User via coordinator |
| 7 | SSH library/executable selection and connection management details | Planner in approved plan |
| 8 | Optional horizon/sky-view satellite display | User product decision |
| 9 | Exact gpsd and chrony telemetry field inventory, units, and mappings | **Resolved** — live-verified against pi@ntp1/ntp2/ntp3; all fields populated in production. |
| 10 | Refresh frequency, reconnect/backoff behavior, stale-data handling | **Timing boxes narrowly resolved (Plans 012–013):** optional SSH date capture times out after 2 s; independent clock/NTP monotonic receipt ages are fresh through 5 s only with status Ok. Failed date capture marks clock unavailable without disabling NTP/GPS; stale/offline samples suppress live digits and needle. Existing 250 ms repaint, 1 s poll sleep, reconnect/history/global-staleness policies remain unchanged and broadly open. |
| 11 | Test coverage thresholds, platform test matrix, CI configuration | User via coordinator |
| 12 | Documentation/versioning/changelog/release conventions | User via coordinator |
| 13 | Dependency admission policy and license rules | User via coordinator |
| 14 | Minimum Linux kernel/glibc version | User via coordinator |
| 15 | Whether orchestration is entirely in `main.rs` or split into `src/app.rs` | Planner in approved plan |
