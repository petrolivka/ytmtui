//! ytmtui - a YouTube Music client for the terminal.

use anyhow::{Context, Result};
use std::io::IsTerminal;
use std::sync::Arc;
use ytm_player::{ResolverCache, YtDlpResolver};

const USAGE: &str = "\
ytmtui - a YouTube Music client for the terminal

USAGE:
    ytmtui [OPTIONS]

OPTIONS:
    -h, --help          show this help
    -V, --version       show the version
        --doctor        check the environment and report what works
        --write-config  write a default config file and exit
        --list-actions  list every bindable action name
        --log-file P    append diagnostics to P (the TUI cannot print them)
        --no-restore    do not restore the previous session
        --anonymous     ignore stored credentials; no account access at all.
                        Use this for any automated or scripted run: a stray
                        keystroke in a signed-in instance writes to the real
                        account.
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    if has("-h") || has("--help") {
        print!("{USAGE}");
        return Ok(());
    }
    if has("-V") || has("--version") {
        println!("ytmtui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if has("--list-actions") {
        // Write through a handle and ignore errors: piping into `head` closes
        // the pipe early, and a broken-pipe panic is not a useful report.
        use std::io::Write;
        let out = std::io::stdout();
        let mut out = out.lock();
        for a in ytm_config::Action::ALL {
            if writeln!(out, "{:<28} {}", a.name(), a.label()).is_err() {
                break;
            }
        }
        return Ok(());
    }
    if has("--write-config") {
        let path = ytm_config::config_path().context("no config directory")?;
        if path.exists() {
            anyhow::bail!("{} already exists; move it aside first", path.display());
        }
        ytm_config::write_default(&path)?;
        println!("wrote {}", path.display());
        return Ok(());
    }
    if has("--doctor") {
        return doctor::run();
    }

    // Diagnostics cannot go to the screen once the TUI owns it.
    if let Some(p) = value("--log-file") {
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
            tracing_subscriber::fmt()
                .with_writer(std::sync::Mutex::new(f))
                .with_ansi(false)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .init();
            tracing::info!("ytmtui {} starting", env!("CARGO_PKG_VERSION"));
        }
    }

    // Restore the terminal even if something panics deep in a widget (NFR-5).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ytm_tui::restore_terminal();
        default_hook(info);
    }));

    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "ytmtui is a full-screen terminal app and needs a real terminal.\n\
             stdin/stdout here are not a TTY - this happens when it is launched from an \n\
             editor or agent shell, or with its output piped. Run it directly in a \n\
             terminal window instead."
        );
    }

    let mut config = ytm_config::load();
    if has("--no-restore") {
        config.config.general.restore_session = false;
    }

    let backend: Arc<dyn ytm_api::MusicBackend> = if has("--anonymous") {
        Arc::new(ytm_api::Innertube::anonymous().context("initialising anonymous client")?)
    } else {
        Arc::new(ytm_api::load_backend().context("initialising YouTube Music client")?)
    };
    let resolver = Arc::new(ResolverCache::new(Arc::new(YtDlpResolver::new(
        config.config.audio.quality.itags(),
    ))));
    let (player, tap) = ytm_player::engine::spawn_on_device(resolver, &config.config.audio.device)
        .context("starting playback engine")?;

    // MPRIS is best-effort: no session bus (a bare TTY, a container, SSH
    // without one) simply means no media-key integration, not a failure to
    // start. The connection must outlive the UI for the bus name to stay
    // claimed, hence the binding.
    #[cfg(target_os = "linux")]
    let _mpris = match ytm_player::mpris::serve(player.clone()) {
        Ok(c) => {
            tracing::info!("MPRIS published as org.mpris.MediaPlayer2.ytmtui");
            Some(c)
        }
        Err(e) => {
            tracing::warn!("MPRIS unavailable: {e}");
            None
        }
    };

    ytm_tui::run(backend, player, tap, config)
}

mod doctor {
    use anyhow::Result;
    use std::process::Command;

    fn tool(bin: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(bin).args(args).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().next().unwrap_or_default().trim().to_string();
        if line.is_empty() {
            let e = String::from_utf8_lossy(&out.stderr);
            return Some(e.lines().next().unwrap_or("present").trim().to_string());
        }
        Some(line)
    }

    fn ok(label: &str, detail: String) {
        println!("  \u{2713} {label:<22} {detail}");
    }
    fn bad(label: &str, detail: &str) {
        println!("  \u{2717} {label:<22} {detail}");
    }

    /// Report what works and what does not, so a broken setup is diagnosed
    /// rather than guessed at.
    pub fn run() -> Result<()> {
        println!("ytmtui {} \u{2014} environment check\n", env!("CARGO_PKG_VERSION"));

        println!("required tools:");
        match tool("ffmpeg", &["-version"]) {
            Some(v) => ok("ffmpeg", v),
            None => bad("ffmpeg", "NOT FOUND - decoding will not work"),
        }
        match tool("yt-dlp", &["--version"]) {
            Some(v) => ok("yt-dlp", v),
            None => bad("yt-dlp", "NOT FOUND - streams cannot be resolved"),
        }

        println!("\naudio:");
        match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(h) => ok("default device", format!("{:?}", h.config())),
            Err(e) => bad("default device", &format!("{e}")),
        }
        let devices = ytm_player::output_devices();
        if devices.is_empty() {
            bad("devices", "none enumerated");
        } else {
            ok("devices", format!("{} available", devices.len()));
            for d in &devices {
                println!("      {d}");
            }
            println!("      (set one with audio.device in config.toml)");
        }

        println!("\nterminal:");
        ok("TERM", std::env::var("TERM").unwrap_or_else(|_| "<unset>".into()));
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            ok("colour", "truecolor".into());
        } else {
            bad("colour", "no truecolor detected - the spectrum gradient will band");
        }

        println!("\nconfig:");
        let loaded = ytm_config::load();
        match &loaded.path {
            Some(p) if p.exists() => ok("config file", p.display().to_string()),
            Some(p) => ok("config file", format!("{} (not present, using defaults)", p.display())),
            None => bad("config file", "no config directory"),
        }
        ok("bindings", format!("{} active", loaded.keymap.len()));
        for w in &loaded.warnings {
            bad("config warning", w);
        }

        println!("\naccount:");
        match ytm_api::load_backend() {
            Ok(b) => {
                if ytm_api::MusicBackend::is_authenticated(&b) {
                    match b.account_name() {
                        Ok(Some(n)) => ok("signed in as", n),
                        Ok(None) => ok("signed in", "yes (name not in response)".into()),
                        Err(e) => bad("signed in", &format!("cookies present but rejected: {e}")),
                    }
                } else {
                    ok("mode", "anonymous - search and playback only".into());
                }
            }
            Err(e) => bad("account", &format!("{e}")),
        }
        Ok(())
    }
}
