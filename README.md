<div align="center">

# ytmtui

**A YouTube Music client for the terminal, with a live audio spectrum where the album art would be.**

[![CI](https://github.com/petrolivka/ytmtui/actions/workflows/ci.yml/badge.svg)](https://github.com/petrolivka/ytmtui/actions/workflows/ci.yml)
[![Licence: GPL-3.0-or-later](https://img.shields.io/badge/licence-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

</div>

<img width="1409" height="848" alt="image" src="https://github.com/user-attachments/assets/92093e0b-d4ab-491f-b473-19b4639d3a13" />


Home feed, Explore, your library, artist/album/playlist pages, search, queue,
radio and autoplay, thumbs up/down — the official player's shape, in a
terminal. Instead of a static cover square, the space is given to a real-time
spectrum analyser driven by the audio actually being played. Album art is
there too, if you want both.

---

## ⚠️ Read this before signing in

**This is an unofficial client.** It uses YouTube's private InnerTube API, which
may place it outside YouTube's Terms of Service, and it can break at any time
when Google changes something.

Account features — your library, thumbs up/down — require **cookie
authentication**; OAuth no longer works for this surface. Using your account
with third-party tooling carries a **real, documented risk of temporary or
permanent account restriction**. A throwaway account is not a workaround, since
the whole point is that your likes reach your real account.

To keep the traffic pattern honest, ytmtui behaves like a player rather than a
scraper: one stream at a time plus a single next-track prefetch, no bulk
fetching, cached responses, rate-limited requests, and **no download or export
feature**.

You can also run it **fully anonymously** — search and playback work with no
credentials at all, with account features disabled. That is the default if you
never sign in, and `--anonymous` forces it.

## Features

| | |
|---|---|
| **Browse** | Home feed, Explore, Charts, New releases |
| **Library** | Liked songs, playlists, albums, artists, history |
| **Pages** | Artist, album and playlist pages with back-navigation |
| **Search** | Songs, albums, artists and playlists, with suggestions and history |
| **Playback** | Gapless, crossfade, seek, 0.5–2× speed with pitch preserved, loudness levelling |
| **Queue** | Reorder-free editing, shuffle, repeat, radio and autoplay |
| **Account** | Thumbs up/down, add to library, playlist editing, subscribe |
| **Visualiser** | Bars, mirrored, oscilloscope, spectrogram; beat-driven accents |
| **Album art** | Half-block (works anywhere), sixel, or the Kitty protocol |
| **Lyrics** | Plain from YouTube Music, time-synced from LRCLIB |
| **Integration** | MPRIS2, media keys, desktop notifications, ListenBrainz, a control socket |
| **Interface** | Themes, fully remappable keys, command palette, mouse, session restore |

## Requirements

| | |
|---|---|
| Rust | 1.88+ to build |
| **`ffmpeg`** | **required** — does all audio decoding |
| **`yt-dlp`** | **required** — resolves stream URLs |
| Audio | PipeWire / PulseAudio / ALSA on Linux, CoreAudio on macOS |

Why external binaries: ffmpeg decodes Opus, which the pure-Rust stack still
cannot, and yt-dlp absorbs YouTube-side breakage far faster than this project
could. The reasoning is in [the technology risk
analysis](docs/TECH-STACK-RISK-ANALYSIS.md).

Run `ytmtui --doctor` to check all of this at once.

## Installing

```bash
cargo binstall ytmtui           # prebuilt binary, once released
cargo install --path .          # from a checkout
```

Packaging recipes for Arch and Homebrew live in
[`contrib/packaging/`](contrib/packaging). Both declare `ffmpeg` and `yt-dlp` as
runtime dependencies, because an install without them cannot decode or resolve
anything.

## Building from source

```bash
git clone https://github.com/petrolivka/ytmtui
cd ytmtui
cargo build --release
./target/release/ytmtui
```

## Signing in (optional)

Export cookies for `music.youtube.com` while signed in, in Netscape format — a
browser extension such as "Get cookies.txt LOCALLY" produces this — then:

```bash
mkdir -p ~/.config/ytmtui
cp ~/Downloads/music.youtube.com_cookies.txt ~/.config/ytmtui/cookies.txt

./target/release/authcheck                     # verify the read path
./target/release/authcheck --like <videoId>    # verify writes, then undo
```

The jar must contain `__Secure-3PAPISID`, which is used to compute the
`SAPISIDHASH` request signature. The file's permissions are tightened to 0600 on
load. **Credentials never leave your machine** and are never logged.

## Configuration

```bash
ytmtui --write-config     # commented default at ~/.config/ytmtui/config.toml
ytmtui --list-actions     # every bindable action name
ytmtui --doctor           # check tools, audio devices, colour, account
```

The config covers autoplay, audio quality, output device, crossfade, playback
speed, the visualiser, album art, scrobbling, colours and key bindings — and is
reloaded when you save it. Every key maps to a named action, so rebinding is one
line:

```toml
[keys]
"ctrl+n" = "next"
"f1"     = "help"
```

Ready-made themes are in [`contrib/themes/`](contrib/themes). A broken entry is
reported at startup and skipped rather than throwing the whole config away.

## Keys

| Key | Action | | Key | Action |
|---|---|---|---|---|
| `/` | search | | `+` / `l` | thumbs up (toggles) |
| `Enter` | open: play, or descend into album/artist/playlist | | `-` / `d` | thumbs down (toggles, skips) |
| `Esc` | back | | `a` | add to / remove from library |
| `Tab` / `Shift-Tab` | cycle sidebar / content / queue | | `s` / `r` | shuffle / repeat |
| `[` `]` | previous / next search tab | | `R` / `A` | radio from selection / autoplay |
| `g` / `G` | go to artist / album | | `9` / `0` | volume down / up |
| `o` / `e` | play next / queue at end | | `v` / `z` | visualiser style / fullscreen |
| `x` | remove from queue | | `L` / `c` | lyrics / album art |
| `:` | command palette | | `P` / `N` | add to playlist / new playlist |
| `Space`, `n`/`p`, `←`/`→` | play/pause, next/prev, seek (Shift: 30s) | | `?` / `q` | help / quit |

`?` shows the live keymap, including anything you have rebound.

Thumbs up adds to **Liked Songs**; it does *not* add to your library. Those are
genuinely different operations in YouTube Music, and ytmtui keeps them distinct.

## Scripting

A running instance listens on a control socket, so a status bar or a keybinding
can drive it:

```bash
ytmtui status --json    # {"state":"playing","title":"Roygbiv",...}
ytmtui next             # also: prev playpause play pause stop shuffle repeat
ytmtui seek -10         # seconds, relative
ytmtui volume 0.4       # omit the value to read it back
```

MPRIS2 is published as well, so `playerctl` and media keys work without any of
this.

## Album art in a terminal

Three backends, picked automatically:

| Terminal | Backend |
|---|---|
| kitty, ghostty, WezTerm | Kitty graphics protocol |
| foot, contour, mlterm | sixel |
| **anything else, including Alacritty** | **half blocks** |

Half blocks work everywhere: a cell's foreground and background become two
stacked pixels via `▀`, and because a cell is about twice as tall as it is wide,
those pixels come out square. Override with `art.backend` if the guess is wrong.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `ytmtui is a full-screen terminal app and needs a real terminal` | Launched from an editor, an agent shell, or with output piped. Run it in a terminal window. |
| Nothing plays, `resolve failed` | `yt-dlp` missing or out of date — `pip install -U yt-dlp`. Check with `--doctor`. |
| Silence, or `ffmpeg:` in the status bar | `ffmpeg` missing. The status bar shows its stderr rather than swallowing it. |
| `session expired or rejected` | Cookies are stale; re-export them while signed in. |
| The spectrum gradient bands | No truecolor. `--doctor` reports what it detected. |
| Album art is a blank pane | The terminal was guessed wrong; set `art.backend = "halfblock"`. |
| Library empty, no LIBRARY section | Running anonymously. `--doctor` says whether you are signed in. |

Diagnostics cannot be printed over a full-screen UI, so use `--log-file`:

```bash
ytmtui --log-file /tmp/ytmtui.log
```

## Project layout

```
crates/ytm-core     domain types (no I/O)
crates/ytm-viz      FFT, banding, smoothing, peaks, onsets (no I/O)
crates/ytm-api      InnerTube: search, browse, auth, ratings, caching; LRCLIB; ListenBrainz
crates/ytm-player   stream resolution, ffmpeg decode, PCM tap, queue engine, MPRIS, IPC
crates/ytm-art      album art: half-block, sixel and Kitty backends
crates/ytm-config   config file, themes, keymap, actions
crates/ytm-tui      ratatui views, spectrum widget, navigation
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). In short:

```bash
cargo test --workspace                  # 53 tests; the parser suites need no network
cargo clippy --workspace --all-targets  # must be clean
cargo fmt --all
cargo run --bin probe                   # read-only check of every API surface
cargo run --bin dump-fixtures           # refresh the captured InnerTube responses
```

The parser tests run against committed fixtures rather than the live API, so a
failure means the code broke — **not** that YouTube changed overnight. Fuzz
targets are in [`fuzz/`](fuzz) and need nightly.

Security issues: see [SECURITY.md](SECURITY.md).

## Documentation

The project was designed before it was built, and each milestone is written up
with what was found — including what turned out to be wrong.

- [Analysis & requirements](docs/ANALYSIS-AND-REQUIREMENTS.md) — requirements, architecture, risk register
- [Technology risk analysis](docs/TECH-STACK-RISK-ANALYSIS.md) — why Rust, and why ffmpeg + yt-dlp
- [M0 findings](docs/M0-FINDINGS.md) — the spike that de-risked the stack
- [Publishing checklist](docs/PUBLISHING.md) — what to settle before a first release
- Milestones: [M1](docs/M1-STATUS.md) · [M2](docs/M2-STATUS.md) · [M3](docs/M3-STATUS.md) · [M4](docs/M4-STATUS.md) · [M5](docs/M5-STATUS.md)

## Licence

Copyright © 2026 the ytmtui contributors.

Licensed under the **GNU General Public License, version 3 or later**. See
[LICENSE](LICENSE).

This program is distributed in the hope that it will be useful, but **without
any warranty**; without even the implied warranty of merchantability or fitness
for a particular purpose.

Every dependency is permissively licensed (MIT, Apache-2.0, Zlib, ISC,
Unicode-3.0), so nothing in the tree forces copyleft — GPL-3.0 is a deliberate
choice here rather than an obligation.

**ytmtui is not affiliated with, endorsed by, or connected to YouTube, YouTube
Music or Google.** All trademarks belong to their respective owners.
