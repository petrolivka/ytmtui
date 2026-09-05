//! ytmtui - a YouTube Music client for the terminal.

use anyhow::{bail, Context, Result};
use std::io::IsTerminal;
use std::sync::Arc;
use ytm_player::{ResolverCache, YtDlpResolver};

fn main() -> Result<()> {
    // Restore the terminal even if something panics deep in a widget (NFR-5).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ytm_tui::restore_terminal();
        default_hook(info);
    }));

    // Check for a usable terminal before opening the audio device, so a
    // headless run fails immediately and cleanly rather than half-starting.
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        bail!(
            "ytmtui is a full-screen terminal app and needs a real terminal.\n\
             stdin/stdout here are not a TTY - this happens when it is launched from an \n\
             editor or agent shell, or with its output piped. Run it directly in a \n\
             terminal window instead."
        );
    }

    let backend = Arc::new(ytm_api::load_backend().context("initialising YouTube Music client")?);

    let resolver = Arc::new(ResolverCache::new(Arc::new(YtDlpResolver::default())));
    let (player, tap) = ytm_player::spawn(resolver).context("starting playback engine")?;

    ytm_tui::run(backend, player, tap)
}
