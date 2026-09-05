//! Terminal UI: state, rendering, and the main loop.

pub mod app;
pub mod clipboard;
pub mod keymap;
pub mod nav;
pub mod session;
pub mod spectrum;
pub mod theme;
pub mod ui;

use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use ytm_api::MusicBackend;
use ytm_player::{PlayerHandle, Tap};

pub use app::App;

/// Put the terminal back into a sane state. Exposed so the binary's panic hook
/// can call it without depending on ratatui directly.
pub fn restore_terminal() {
    ratatui::restore();
}

pub fn run(
    backend: Arc<dyn MusicBackend>,
    player: PlayerHandle,
    tap: Tap,
    config: ytm_config::Loaded,
) -> Result<()> {
    let max_fps = config.config.visualizer.max_fps.clamp(15, 144) as u64;
    let mut app = App::new(backend, player, tap, config);
    // `ratatui::init()` panics when there is no controlling terminal; use the
    // fallible form so a headless invocation gets an explanation instead.
    let mut terminal = ratatui::try_init().map_err(|e| {
        anyhow!(
            "could not initialise the terminal: {e}\n\
             ytmtui is a full-screen TUI and needs a real terminal. If you launched it \n\
             from an editor, an agent shell, a pipe, or a cron job, run it directly in a \n\
             terminal window instead."
        )
    })?;
    let frame_budget = Duration::from_millis(1000 / max_fps);

    let res = (|| -> Result<()> {
        while !app.should_quit {
            let t0 = Instant::now();
            terminal.draw(|f| ui::draw(f, &app))?;
            app.tick();
            // Spend whatever is left of the frame waiting for input, so an idle
            // app costs nothing but a keypress is still handled immediately.
            let spent = t0.elapsed();
            let wait = frame_budget.saturating_sub(spent);
            app.poll_input(wait.max(Duration::from_millis(1)))?;
        }
        Ok(())
    })();

    app.shutdown();
    ratatui::restore();
    res
}
