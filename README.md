# ytmtui

A YouTube Music client for the terminal, in Rust + Ratatui — with a live audio
spectrum where the album art would be.

```
╭ spectrum • mirrored • 128 bands ──────────────────────────────────────────────╮
│  ▁▁▁▁▂▆███▇▆▆▄▁▁▁▁▄▆▆▅▄▃▂▁▁▁▁▁▁▁▁▁▁          ▁            ▁▁▁▁▁▁▁▁▁▁         │
│ ▃▅▅▅▆▆▆▆████████████▆▂ ▂█████████▅▃▂▂▄█▇▂ ▄▆▃  ▁▁▁▁▁▁▁▁ ▁▁▂▅██▆▄▂▁ ▁▁▁▁       │
│███████████████████████████████████████████▄▁▃▅▅▂▁▂▄▆▅▂ ▁▂▅██████████▆▄▂ ▁▁    │
│███████████████████████████████████████████████████████████████████████████▆▆  │
╰───────────────────────────────────────────────────────────────────────────────╯
╭ now playing ──────────────────────────────────────────────────────────────────╮
│▶ Xtal  •  Aphex Twin                              repeat:off          vol 100%│
│0:14 ━━━━●─────────────────────────────────────────────────────────────── 4:54 │
╰───────────────────────────────────────────────────────────────────────────────╯
```

**Status: M1 (skeleton player).** Search, play, pause, seek, volume, queue,
shuffle/repeat, thumbs up/down, and the live spectrum all work. Home feed,
Explore, artist/album pages, playlist editing and lyrics are not built yet — see
the [roadmap](docs/ANALYSIS-AND-REQUIREMENTS.md#13-roadmap).

## ⚠️ Read this before signing in

This is an **unofficial** client. It uses YouTube's private InnerTube API, which
may put it outside YouTube's Terms of Service, and it can break at any time.

Account features (your library, thumbs up/down) require **cookie
authentication** — OAuth no longer works for this surface. Using your account
with third-party tooling carries a **real, documented risk of temporary or
permanent account restriction**. A throwaway account is not a workaround here,
because the whole point is that your likes sync back to your real account.

To keep the traffic pattern honest, ytmtui deliberately behaves like a player
and not a scraper: one stream at a time plus a single next-track prefetch, no
bulk fetching, global rate limiting, and **no download or export feature**.

You can also run it fully anonymously — search and playback work with no
credentials at all, with account features disabled.

## Requirements

| | |
|---|---|
| Rust | 1.85+ (`mise.toml` pins a toolchain) |
| `ffmpeg` | **required** — does all audio decoding |
| `yt-dlp` | **required** — resolves stream URLs |
| Audio | PipeWire / PulseAudio / ALSA (Linux), CoreAudio, WASAPI |

Why external binaries: ffmpeg decodes Opus, which the pure-Rust stack still
cannot, and yt-dlp absorbs YouTube-side breakage far faster than we could. The
reasoning is in [docs/TECH-STACK-RISK-ANALYSIS.md](docs/TECH-STACK-RISK-ANALYSIS.md).

## Build and run

```bash
cargo build --release
./target/release/ytmtui
```

## Signing in (optional)

Export cookies for `music.youtube.com` while signed in, in Netscape format
(a browser extension such as "Get cookies.txt LOCALLY" produces this), then:

```bash
mkdir -p ~/.config/ytmtui
cp ~/Downloads/music.youtube.com_cookies.txt ~/.config/ytmtui/cookies.txt
# or:  export YTM_COOKIE='<raw Cookie header value>'

./target/release/authcheck            # verify the read path
./target/release/authcheck --like dQw4w9WgXcQ   # verify writes, then undo
```

The cookie jar must contain `__Secure-3PAPISID`, which is used to compute the
`SAPISIDHASH` request signature. Credentials never leave your machine.

## Keys

| Key | Action | | Key | Action |
|---|---|---|---|---|
| `/` | search | | `+` / `l` | thumbs up (toggles) |
| `Enter` | play selection / jump to | | `-` / `d` | thumbs down (toggles, skips) |
| `o` / `e` | play next / queue at end | | `s` / `r` | shuffle / repeat |
| `x` | remove from queue | | `9` / `0` | volume down / up |
| `Space` | play / pause | | `Tab` | switch pane |
| `n` / `p` | next / previous | | `v` / `z` | visualiser style / fullscreen |
| `←` `→` | seek 5s (Shift: 30s) | | `?` / `q` | help / quit |

Thumbs up adds to **Liked Songs**; it does *not* add to your library. Those are
genuinely different operations in YouTube Music, and ytmtui keeps them distinct.

## Tools

```bash
cargo run --release --bin tune -- <videoId|file> [seconds]
```

Dumps the measured distribution of spectrum band energies. The analyser's
tuning constants were derived from this rather than chosen by eye; re-run it
after changing them.

## Layout

```
crates/ytm-core    domain types (no I/O)
crates/ytm-viz     FFT, banding, smoothing, peaks (no I/O - unit-testable)
crates/ytm-api     InnerTube: search, auth, ratings
crates/ytm-player  stream resolution, ffmpeg decode, PCM tap, queue engine
crates/ytm-tui     ratatui views, spectrum widget, theming, keymap
```

## Documentation

- [Analysis & requirements](docs/ANALYSIS-AND-REQUIREMENTS.md)
- [Technology risk analysis](docs/TECH-STACK-RISK-ANALYSIS.md) — why Rust, and why ffmpeg + yt-dlp
- [M0 spike findings](docs/M0-FINDINGS.md)
- [M1 status](docs/M1-STATUS.md)
