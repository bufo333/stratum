//! Orchestration: runtime lifecycle, event/draw loop, terminal setup/restore.
//!
//! This is the only module that performs terminal I/O.
//! It owns the `Model` and is the sole consumer of `ServerUpdate` messages.

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    Terminal,
};
use tokio::sync::mpsc;

use crate::{
    cli::Cli,
    ingest,
    model::{Model, ServerUpdate},
    render,
};

/// Fallback repaint interval (250 ms) when no events arrive.
const REPAINT_INTERVAL: Duration = Duration::from_millis(250);

/// mpsc channel capacity.
const CHANNEL_CAPACITY: usize = 256;

/// Entry point called from `main`.
///
/// Sets up the tokio multi-thread runtime, spawns ingest tasks, runs the
/// event/draw loop, and restores the terminal on exit (including error paths).
pub async fn run(cli: Cli) -> Result<()> {
    // Validate server count (clap already enforces 1–3, but belt-and-suspenders).
    if cli.servers.is_empty() || cli.servers.len() > 3 {
        anyhow::bail!("Expected 1–3 servers, got {}", cli.servers.len());
    }

    // -----------------------------------------------------------------------
    // Terminal setup
    // -----------------------------------------------------------------------
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the main loop, capturing the result so we can restore the terminal
    // before propagating any error.
    let result = run_loop(&mut terminal, cli).await;

    // -----------------------------------------------------------------------
    // Terminal restore (always executed)
    // -----------------------------------------------------------------------
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Main event/update/draw loop.
async fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, cli: Cli) -> Result<()> {
    // Build model.
    let mut model = Model::new(cli.servers.clone());

    // Bounded mpsc channel: one Sender per ingest task, single Receiver here.
    let (tx, mut rx) = mpsc::channel::<ServerUpdate>(CHANNEL_CAPACITY);

    // Spawn one ingest task per server.
    for (index, hostname) in cli.servers.iter().enumerate() {
        let tx_clone = tx.clone();
        let host = hostname.clone();
        tokio::spawn(async move {
            ingest::run_server_ingest(index, host, tx_clone).await;
        });
    }
    // Drop the original sender so the channel closes when all tasks finish.
    drop(tx);

    loop {
        // -----------------------------------------------------------------------
        // Drain available updates (non-blocking).
        // -----------------------------------------------------------------------
        loop {
            match rx.try_recv() {
                Ok(update) => model.apply(update),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // All ingest tasks exited — this would be unexpected in normal
                    // operation, but treat it as a clean shutdown.
                    return Ok(());
                }
            }
        }

        // -----------------------------------------------------------------------
        // Draw
        // -----------------------------------------------------------------------
        terminal.draw(|f| draw(f, &model))?;

        // -----------------------------------------------------------------------
        // Wait for next event or timeout (poll blocks for up to REPAINT_INTERVAL).
        // -----------------------------------------------------------------------
        if crossterm::event::poll(REPAINT_INTERVAL)? {
            match event::read()? {
                Event::Key(KeyEvent {
                    code: KeyCode::Char('q'),
                    ..
                })
                | Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                }) => {
                    return Ok(());
                }
                Event::Resize(_, _) => {
                    // Force a redraw on the next iteration.
                }
                _ => {}
            }
        }

        // Drain updates that arrived while we were waiting.
        while let Ok(update) = rx.try_recv() {
            model.apply(update);
        }
    }
}

/// Build and render the TUI layout from a model snapshot.
///
/// Layout (top → bottom):
///   - Summary table (20% height)
///   - Graphs area (40% height): offset | freq side by side
///   - Server panels (40% height): one panel per server
fn draw(f: &mut ratatui::Frame, model: &Model) {
    let size = f.area();

    // Top-level vertical split.
    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(size);

    // Summary table.
    render::table::render_summary_table(f, top_chunks[0], model);

    // Graphs: offset | freq.
    let graph_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(top_chunks[1]);

    render::graphs::render_offset_graph(f, graph_chunks[0], model);
    render::graphs::render_freq_graph(f, graph_chunks[1], model);

    // Server panels: one per server, evenly distributed.
    let n = model.server_count().max(1);
    let panel_constraints: Vec<Constraint> =
        (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(panel_constraints)
        .split(top_chunks[2]);

    for (i, server) in model.servers().iter().enumerate() {
        if let Some(chunk) = panel_chunks.get(i) {
            render::panels::render_server_panel(f, *chunk, server);
        }
    }
}

/// Detect whether a server's NTP data has gone stale.
///
/// Extracted as a pure helper to allow unit testing without a running loop.
/// Returns `true` if the history ring buffer is non-empty but the ring buffer
/// has not grown in `threshold` samples worth of time — this is a simplified
/// heuristic; a real implementation would compare wall-clock timestamps on
/// samples.
#[allow(dead_code)]
pub fn is_stale(_sample_count_a: usize, _sample_count_b: usize) -> bool {
    // Placeholder: stale detection is driven by the orchestration loop
    // comparing sample timestamps.  Returns false until timestamps are added
    // to `NtpSample` (contract Q10 pending).
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stale_placeholder() {
        assert!(!is_stale(0, 0));
        assert!(!is_stale(5, 3));
    }
}
