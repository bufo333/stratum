# Project Architecture — trmnlapp

## Overview

trmnlapp is a read-only TUI monitoring application. It collects diagnostics from
up to six LAN servers over SSH and displays NTP/chrony timing statistics and
gpsd satellite/GPS data in a terminal UI. The architecture separates collection,
state, view composition, and orchestration into distinct concerns with enforced
boundaries.

## Four-Concern Model

The application is structured around four concerns. Three of these are data
layers with strict boundary rules; the fourth is the orchestration concern that
owns lifecycle and I/O coordination.

```
┌─────────────────────────────────────────────────────────────────┐
│  LAN servers (up to 6): chrony + gpsd                           │
└────────────────────────┬────────────────────────────────────────┘
                         │ read-only diagnostics
                         │ (SSH preferred; mechanism is a planning decision)
                         ▼
┌────────────────────────────────────────────────────────────────┐
│  INGEST                                                        │
│  Owns: SSH auth integration, collection I/O, telemetry parsing │
│  Sends: normalized update messages                             │
│  Must not: mutate shared model state directly, import widgets, │
│            invoke rendering                                    │
└────────────────────────┬───────────────────────────────────────┘
                         │ normalized updates (channel type TBD)
                         ▼
┌────────────────────────────────────────────────────────────────┐
│  MODEL                                                         │
│  Owns: per-server current state, in-memory ring-buffer history │
│  Provides: read-only snapshots on request                      │
│  Must not: perform network, subprocess, or file I/O; import    │
│            ingest or widget modules; render anything           │
└────────────────────────┬───────────────────────────────────────┘
                         │ read-only snapshot
                         ▼
┌────────────────────────────────────────────────────────────────┐
│  WIDGETS (src/render/ or src/ui/)                              │
│  Owns: view composition — ratatui layout/widget tree only      │
│  Must not: perform terminal I/O, event handling, filesystem or │
│            network I/O; mutate model state; call ingest        │
└────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────┐
│  ORCHESTRATION (main.rs or src/app.rs)                         │
│  Owns: CLI arg parsing, tokio runtime setup, ingest task       │
│        lifecycle, update-consumer loop, terminal               │
│        setup/restoration, crossterm event handling, draw       │
│        scheduling, shutdown/cleanup                            │
│  Drives: drawing by passing model snapshots to widgets and     │
│          writing the result to the terminal                    │
│  Must not: contain telemetry parsing, model transitions, or    │
│            widget composition logic                            │
└────────────────────────────────────────────────────────────────┘
```

Orchestration is not a fourth data layer — it does not sit between ingest,
model, and widgets in the data-flow sense. It is the runtime owner: it starts
and stops ingest tasks, consumes updates from ingest and applies them to the
model, reads model snapshots, passes them to widgets, and performs the terminal
I/O. This resolves the prior conflict where the render layer was described as
both "widgets only / no I/O" and responsible for the terminal/event loop.

## Layer Rules

### Ingest

- Owns all read-only collection I/O and SSH authentication integration.
- Owns telemetry parsing; service-specific parsing is encapsulated within ingest
  boundaries and not visible to other layers.
- Sends normalized update messages to orchestration; does not write to shared
  model state directly.
- Does not import widget or model-mutation interfaces.
- Must be independently restartable per server without affecting the UI.
- Timeout, backoff, and cancellation policy are open questions; see contract.

### Model

- Owns per-server current state and in-memory history (fixed-capacity ring
  buffers; capacity is an open question).
- Exposes transitions (for orchestration to apply updates) and read-only
  snapshots (for orchestration to pass to widgets).
- Does not depend on ingest or widget implementation modules.
- No network, subprocess, or file I/O.
- Represents only verified metrics; exact field names, units, and time basis
  must be verified during planning against actual gpsd JSON and chronyc output.

### Widgets

- View composition only: consume model snapshots and produce ratatui
  layout/widget trees.
- No terminal I/O, no crossterm event handling, no filesystem or network I/O.
- No model mutations; no calls into ingest.
- Layout proposal (pending user approval): combined color-coded multi-server
  graphs (one line per server per metric), per-server detail panels, and a
  summary table. Optional horizon/sky-view display is an open product question.

### Orchestration

- Entry point: `main.rs`, optionally delegating to a `src/app.rs` module
  (planning decision).
- Parses CLI positional arguments (server hostnames/IPs, one to six; zero or
  more than six is a usage error).
- Sets up the tokio multi-thread runtime.
- Spawns and manages per-server ingest tasks.
- Consumes update messages from ingest and applies model transitions.
- Sets up and restores the terminal (raw mode, alternate screen).
- Handles crossterm input events (resize, quit, navigation).
- Schedules and drives draw calls: reads a model snapshot, passes it to widgets,
  writes the resulting frame to the terminal.
- Manages graceful shutdown and cleanup.
- Does not hold a shared-state lock across terminal drawing, network I/O, or
  waits (exact snapshot/locking mechanism is a planning decision).

## Module Map (Planning Decisions)

Concrete paths below are candidates to be confirmed in the approved plan.
No path is created or frozen by this document. A planner must select and assign
module ownership before implementation begins; implementers may not improvise
structural changes.

| Candidate path | Responsibility |
| --- | --- |
| `src/main.rs` | Orchestration entry point; CLI parsing, runtime setup, terminal loop, draw scheduling |
| `src/app.rs` | Optional orchestration split (planning decision) |
| `src/ingest/mod.rs` | Collection/update interface and shared ingest types |
| `src/ingest/ssh.rs` | SSH transport layer (mechanism is a planning decision) |
| `src/ingest/gpsd.rs` | gpsd telemetry adapter; exact fields verified during planning |
| `src/ingest/ntp.rs` | NTP/chrony diagnostics adapter; mechanism verified during planning |
| `src/model/mod.rs` | State ownership, update transitions, snapshot API |
| `src/model/server.rs` | Per-server state structure |
| `src/model/ring_buffer.rs` | Fixed-capacity in-memory history; capacity is an open question |
| `src/render/mod.rs` or `src/ui/mod.rs` | Widget composition root; path is a planning decision |
| `src/render/graphs.rs` | Multi-server color-coded timing graphs |
| `src/render/panels.rs` | Per-server detail panels |
| `src/render/table.rs` | Summary table widget |

`src/ui/` is an equally valid alternative to `src/render/`. The planner chooses
one and records it in the approved plan.

## Approved Integrated Timing Ownership (Plans 012–013)

- `src/ingest/clock.rs` detects optional `date +%s.%N` support over the existing
  SSH session, validates integer epoch nanoseconds and normalizes capture midpoint,
  receipt time and RTT. The query has a two-second timeout.
- `src/ingest/ntp.rs` joins clock and NTP collection within each existing poll;
  each publishes immediately on completion. Clock failure is optional; NTP
  failure retains the existing reconnect behavior. No extra session or detached
  clock task is introduced.
- `src/model/server.rs` owns the latest optional capture and NTP provenance and
  receipt timestamp. Pure projection/freshness helpers receive explicit monotonic
  time; model and widgets never acquire clocks themselves.
- `src/render/mod.rs` owns the single dashboard composition, timing-strip sizing,
  dashboard-first small-terminal fallback and controls/geometry footer.
  `src/render/clocks.rs` reads model snapshots for five-row cards with ordinary
  UTC digits, offset needles, light equivalents and independent receipt ages.
- `src/app.rs` owns quit/resize input and obtains one monotonic timestamp per
  draw. It delegates widget composition to the render entry point.

## Concurrency Model (Planning Decisions)

The following describes the intended shape; exact channel types, lock primitives,
and scheduling cadence are planning decisions:

- tokio multi-thread scheduler.
- One ingest task per server (independently restartable).
- Ingest tasks send updates over an async channel (e.g., `mpsc` or `broadcast`;
  type is a planning decision) to the orchestration loop.
- Orchestration applies updates to the model via transitions.
- Before each draw, orchestration reads a short-lived model snapshot; it does
  not hold a lock across the draw call or any I/O.

## SSH and Known-Hosts Policy

- Use the user's existing SSH configuration, keys in `~/.ssh/`, and
  `known_hosts` trust data.
- TOFU applies for genuinely new hosts: accept and record to `known_hosts`.
- Changed known-host identities must not be silently accepted; treat as an
  error.
- Do not copy, store, or log private-key material or passwords.
- Recording a new host in `known_hosts` is the only permitted persistent write;
  it does not authorize telemetry persistence.
- SSH library/executable selection and remote command invocation are planning
  decisions.

## Extension Points

The following are identified future extension points. No stubs, reserved module
slots, or implementation work are authorized until scope is confirmed and a plan
is approved.

| Extension | Status |
| --- | --- |
| PPS device support (`/dev/pps*`) | Future extension point; out of current scope |
| TimeClick serial/USB | Out of current scope; potential future extension |
| Direct gpsd socket/TCP (non-SSH) | Collection mechanism candidate; not decided |
| Direct NTP protocol (non-SSH) | Collection mechanism candidate; not decided |

Extensions follow the same ingest/model/widget boundary rules when implemented.

## Open Questions

Items requiring resolution before dependent implementation begins:

1. SSH library vs. executable selection and connection management details.
2. Exact gpsd telemetry field inventory, units, and available JSON fields.
3. Exact NTP/chrony query mechanism (subprocess vs. SSH remote command vs.
   direct protocol).
4. Maximum ring buffer size and history retention duration.
5. Server color palette and accessibility/color policy.
6. Optional horizon/sky-view satellite display (product decision).
7. Refresh frequency, reconnect/backoff, stale-data handling.
8. Whether orchestration splits into `src/app.rs` or stays in `main.rs`.
9. Exact macOS architecture and minimum OS version.
10. Test platform matrix and CI configuration.

Full open-question register is in `docs/engineering-contract.md`, Section 11.
