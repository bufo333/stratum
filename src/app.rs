//! Orchestration: runtime lifecycle, event/draw loop, terminal setup/restore.
//!
//! This is the only module that performs terminal I/O.
//! It owns the `Model` and is the sole consumer of `ServerUpdate` messages.

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
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
    // Validate server count (clap already enforces 1–6, but belt-and-suspenders).
    if cli.servers.is_empty() || cli.servers.len() > 6 {
        anyhow::bail!("Expected 1–6 servers, got {}", cli.servers.len());
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
    let mut middle_view = render::MiddleView::default();

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
        let now = Instant::now();
        terminal.draw(|f| render::draw(f, &model, middle_view, now))?;

        // -----------------------------------------------------------------------
        // Wait for next event or timeout (poll blocks for up to REPAINT_INTERVAL).
        // -----------------------------------------------------------------------
        if crossterm::event::poll(REPAINT_INTERVAL)? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(key, &mut middle_view) {
                        return Ok(());
                    }
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

fn handle_key(key: KeyEvent, view: &mut render::MiddleView) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }
    match key.code {
        KeyCode::Char('q') => true,
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => true,
        KeyCode::Tab if key.kind == KeyEventKind::Press => {
            *view = match view {
                render::MiddleView::Graphs => render::MiddleView::Timing,
                render::MiddleView::Timing => render::MiddleView::Graphs,
            };
            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_toggle_middle_and_quit_in_both_modes() {
        let mut view = render::MiddleView::default();
        assert_eq!(view, render::MiddleView::Graphs);
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        for expected in [render::MiddleView::Timing, render::MiddleView::Graphs] {
            assert!(!handle_key(tab, &mut view));
            assert_eq!(view, expected);
            for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
                assert!(!handle_key(KeyEvent { kind, ..tab }, &mut view));
                assert_eq!(view, expected);
            }
            assert!(handle_key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                &mut view
            ));
            assert!(handle_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &mut view
            ));
            assert!(!handle_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
                &mut view
            ));
        }
    }
}
