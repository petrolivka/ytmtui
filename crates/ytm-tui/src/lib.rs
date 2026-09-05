//! Terminal UI: state, rendering, and the main loop.

pub mod app;
pub mod clipboard;
pub mod cover;
pub mod keymap;
pub mod modal;
pub mod nav;
pub mod notify;
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

/// Sixel and the Kitty protocol are raw escapes, so they are written after the
/// frame, positioned by hand over cells the renderer was told to skip.
fn draw_graphics_cover(app: &App) -> Result<()> {
    use std::io::Write;

    // Fullscreen draws the spectrum alone. Without this the image would keep
    // being painted at the position the cover pane had before, because these
    // escapes bypass the renderer and nothing else erases them.
    // Every path that declines to draw must both forget what was painted and,
    // on Kitty, actively remove it. A sixel image is pixels in the grid that
    // the next text write covers up; a Kitty placement is an object that
    // survives the redraw and stays until it is deleted.
    let forget = || -> Result<()> {
        let had = app.hit.painted.borrow_mut().take();
        if had.is_some() && app.art_backend == ytm_art::Backend::Kitty {
            let mut out = std::io::stdout().lock();
            out.write_all(ytm_art::kitty_delete().as_bytes())?;
            out.flush()?;
        }
        Ok(())
    };

    if !app.show_art || !cover::is_graphics(app.art_backend) {
        return forget();
    }
    if app.viz_fullscreen || app.modal.is_some() || app.show_help {
        return forget();
    }
    let area = app.hit.cover.get();
    if area.width == 0 || area.height == 0 {
        return forget();
    }
    let Some(url) = app
        .player
        .status()
        .current
        .as_ref()
        .and_then(|t| t.thumbnail.clone())
    else {
        return forget();
    };
    let want = ytm_art::at_size(
        &url,
        (area.width as u32)
            .max(area.height as u32 * 2)
            .clamp(64, 544),
    );
    // Redraw only when the image or its position actually changed. These
    // escapes are not part of the frame diff, so nothing erases them in the
    // meantime and repeating them every frame buys nothing.
    //
    // This has to come before the cache lookup, not after: `get` hands back an
    // owned copy of the decoded cover, so asking first and checking second
    // meant cloning the whole image every frame only to discard it.
    if app
        .hit
        .painted
        .borrow()
        .as_ref()
        .is_some_and(|(painted, at)| painted == &want && *at == area)
    {
        return Ok(());
    }
    let Some(img) = app.art_cache.get(&want) else {
        return Ok(());
    };

    let mut out = std::io::stdout().lock();
    // Remove the previous placement first. Re-transmitting under the same id
    // does not move an existing one, so after a resize the old placement would
    // otherwise stay where it was.
    if app.art_backend == ytm_art::Backend::Kitty {
        out.write_all(ytm_art::kitty_delete().as_bytes())?;
    }
    // Save the cursor, position it over the pane, draw, restore.
    write!(out, "\x1b7\x1b[{};{}H", area.y + 1, area.x + 1)?;
    match app.art_backend {
        ytm_art::Backend::Kitty => {
            if let Ok(seq) = ytm_art::to_kitty(&img, area.width, area.height) {
                out.write_all(seq.as_bytes())?;
            }
        }
        ytm_art::Backend::Sixel => {
            let scaled = ytm_art::resize_for_cells(&img, area.width, area.height);
            out.write_all(ytm_art::sixel::encode(&scaled).as_bytes())?;
        }
        _ => {}
    }
    write!(out, "\x1b8")?;
    out.flush()?;
    *app.hit.painted.borrow_mut() = Some((want, area));
    Ok(())
}

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
    let mut mouse_on = false;
    let mut terminal = ratatui::try_init().map_err(|e| {
        anyhow!(
            "could not initialise the terminal: {e}\n\
             ytmtui is a full-screen TUI and needs a real terminal. If you launched it \n\
             from an editor, an agent shell, a pipe, or a cron job, run it directly in a \n\
             terminal window instead."
        )
    })?;
    let frame_budget = Duration::from_millis(1000 / max_fps);

    // Mouse support is optional: a terminal that refuses it still works fully
    // from the keyboard.
    if ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableMouseCapture
    )
    .is_ok()
    {
        mouse_on = true;
    }

    let res = (|| -> Result<()> {
        while !app.should_quit {
            let t0 = Instant::now();
            terminal.draw(|f| ui::draw(f, &app))?;
            draw_graphics_cover(&app)?;
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
    // A Kitty placement outlives the alternate screen on some terminals, so
    // leaving one behind would put an album cover in the user's shell.
    if app.art_backend == ytm_art::Backend::Kitty {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(ytm_art::kitty_delete().as_bytes());
        let _ = out.flush();
    }
    if mouse_on {
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::DisableMouseCapture
        );
    }
    ratatui::restore();
    res
}
