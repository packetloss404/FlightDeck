use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::Terminal;

use crate::core::pty::PtyEvent;
use crate::core::storage;

mod app;
mod command_palette;
mod core;
mod theme;
mod views;
mod widgets;

use app::App;

fn main() -> io::Result<()> {
    // Initialize tracing to file (not stdout — we own the terminal)
    init_file_tracing();

    // Load persisted state
    let persisted = storage::load_state();

    // Create PTY event channel
    let (pty_tx, pty_rx) = mpsc::channel::<PtyEvent>();

    // Create the app
    let mut app = App::new(
        persisted.flights,
        persisted.agents,
        persisted.settings,
        persisted.ui,
        pty_tx,
        pty_rx,
    );

    // Hydrate retrospectives from persisted state
    app.retrospectives = persisted.retrospectives;

    // Detect installed agents
    app.detect_agents();
    app.refresh_git_context();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Main loop
    let result = run_loop(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Save state on exit
    let _ = app.persist_state();

    result
}

fn run_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    let mut last_agent_check = Instant::now();
    let mut last_git_refresh = Instant::now();
    const AGENT_POLL_INTERVAL: Duration = Duration::from_secs(30);
    const GIT_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

    loop {
        terminal.draw(|f| app.render(f))?;

        // Poll for events with 50ms timeout (20fps effective)
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        return Ok(());
                    }
                    if app.handle_key(key) {
                        return Ok(()); // quit requested
                    }
                }
                _ => {}
            }
        }

        // Expire leader key if timed out
        if let Some(started) = app.leader_pending {
            if started.elapsed() > Duration::from_secs(1) {
                app.leader_pending = None;
            }
        }

        // Expire toasts
        app.toasts.gc();

        // Drain PTY events
        app.poll_pty_events();

        // Orchestrator tick
        app.orchestrator_tick();

        // Periodic agent detection (every 30s)
        if last_agent_check.elapsed() >= AGENT_POLL_INTERVAL {
            app.detect_agents();
            last_agent_check = Instant::now();
        }

        // Periodic git context refresh (every 60s)
        if last_git_refresh.elapsed() >= GIT_REFRESH_INTERVAL {
            app.refresh_git_context();
            last_git_refresh = Instant::now();
        }
    }
}

fn init_file_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let log_dir = crate::core::storage::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let file_appender = tracing_appender::rolling::daily(log_dir, "packetcode-tui.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    std::mem::forget(guard);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();
}
