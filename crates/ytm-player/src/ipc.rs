//! A control socket, so a running player can be driven from scripts and status
//! bars without a D-Bus stack.
//!
//! Line-based and deliberately tiny: one command per line, one line of reply.

use crate::engine::{Command, PlayerHandle};
use anyhow::Result;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use ytm_core::PlayState;

pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("ytmtui.sock")
}

/// Serve the control socket until the process exits.
pub fn serve(handle: PlayerHandle) -> Result<()> {
    let path = socket_path();
    // A socket left behind by a crash would block binding forever.
    if path.exists() && UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)?;
    std::thread::Builder::new()
        .name("ytm-ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let h = handle.clone();
                std::thread::spawn(move || {
                    let _ = handle_client(stream, h);
                });
            }
        })?;
    Ok(())
}

fn handle_client(stream: UnixStream, handle: PlayerHandle) -> Result<()> {
    let mut out = stream.try_clone()?;
    let reader = BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        let reply = dispatch(&handle, line.trim());
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}

/// Handle one command, returning the single-line reply.
pub fn dispatch(handle: &PlayerHandle, line: &str) -> String {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next();
    match cmd {
        "status" => status_json(handle),
        "playpause" | "toggle" => {
            handle.send(Command::TogglePause);
            "ok".into()
        }
        "play" => {
            if handle.status().state == PlayState::Paused {
                handle.send(Command::TogglePause);
            }
            "ok".into()
        }
        "pause" => {
            if handle.status().state == PlayState::Playing {
                handle.send(Command::TogglePause);
            }
            "ok".into()
        }
        "next" => {
            handle.send(Command::Next);
            "ok".into()
        }
        "prev" | "previous" => {
            handle.send(Command::Prev);
            "ok".into()
        }
        "stop" => {
            handle.send(Command::Stop);
            "ok".into()
        }
        "seek" => match arg.and_then(|a| a.parse::<f64>().ok()) {
            Some(v) => {
                handle.send(Command::SeekRelative(v));
                "ok".into()
            }
            None => "error: seek needs seconds, e.g. `seek -10`".into(),
        },
        "volume" => match arg.and_then(|a| a.parse::<f32>().ok()) {
            Some(v) => {
                handle.send(Command::SetVolume(v));
                "ok".into()
            }
            None => format!("{:.2}", handle.status().volume),
        },
        "speed" => match arg.and_then(|a| a.parse::<f32>().ok()) {
            Some(v) => {
                handle.send(Command::SetSpeed(v));
                "ok".into()
            }
            None => format!("{:.2}", handle.status().speed),
        },
        "shuffle" => {
            handle.send(Command::ToggleShuffle);
            "ok".into()
        }
        "repeat" => {
            handle.send(Command::CycleRepeat);
            "ok".into()
        }
        "" => "error: empty command".into(),
        other => format!("error: unknown command '{other}'"),
    }
}

fn status_json(handle: &PlayerHandle) -> String {
    let s = handle.status();
    let esc = |v: &str| {
        // Enough for JSON string bodies; track titles really do contain quotes.
        v.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    };
    let (title, artist, album, id, duration) = match &s.current {
        Some(t) => (
            esc(&t.title),
            esc(&t.artist),
            esc(t.album.as_deref().unwrap_or("")),
            t.id.0.clone(),
            t.duration.map(|d| d.as_secs()).unwrap_or(0),
        ),
        None => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            0,
        ),
    };
    format!(
        concat!(
            r#"{{"state":"{}","title":"{}","artist":"{}","album":"{}","id":"{}","#,
            r#""position":{:.1},"duration":{},"volume":{:.2},"speed":{:.2},"#,
            r#""shuffle":{},"repeat":"{}","queue":{},"index":{}}}"#
        ),
        match s.state {
            PlayState::Playing => "playing",
            PlayState::Paused => "paused",
            PlayState::Buffering => "buffering",
            PlayState::Stopped => "stopped",
        },
        title,
        artist,
        album,
        id,
        s.position.as_secs_f64(),
        duration,
        s.volume,
        s.speed,
        s.shuffle,
        s.repeat.glyph(),
        s.queue.len(),
        s.queue_index,
    )
}

/// Send one command to a running instance and return its reply.
pub fn send(line: &str) -> Result<String> {
    let mut stream = UnixStream::connect(socket_path())?;
    writeln!(stream, "{line}")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    Ok(reply.trim_end().to_string())
}
