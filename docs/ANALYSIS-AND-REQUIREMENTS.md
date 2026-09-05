# ytmtui — Analysis & Requirements

**A YouTube Music client for the terminal, in Rust + Ratatui, with a live audio spectrum where the artwork would be.**

| | |
|---|---|
| Document status | Draft v0.1 — for review |
| Date | 2026-09-04 |
| Owner | petr |
| Scope | Product analysis, feature brainstorm, functional & non-functional requirements, architecture proposal, roadmap |
| Out of scope | Implementation, detailed API schemas, test plans |
| M3 results | [M3-STATUS.md](./M3-STATUS.md) — **polish complete**; all `S` requirements met bar two, listed with reasons |
| M2 results | [M2-STATUS.md](./M2-STATUS.md) — **parity core complete**; a listening session needs no browser |
| M1 results | [M1-STATUS.md](./M1-STATUS.md) — **skeleton player complete**; spike promoted to `crates/` |
| M0 results | [M0-FINDINGS.md](./M0-FINDINGS.md) — **spike complete, verdict GO**; supersedes several assumptions below |
| Companion doc | [TECH-STACK-RISK-ANALYSIS.md](./TECH-STACK-RISK-ANALYSIS.md) — evaluates non-Rust stacks; **its §5/§8 revise the decode and stream-resolution decisions below** |

---

## 1. Vision

> A keyboard-driven YouTube Music that feels like the official player — same library, same likes, same radio — but renders as a fast, colourful TUI, and replaces the album-art square with a real-time spectrum analyser driven by the actual audio being played.

Three things define the product:

1. **Parity, not a subset.** The mental model is the official web/mobile player: Home feed, Explore, Library, search, queue, radio/autoplay, thumbs up/down, add-to-library, playlists, lyrics. A user should not have to open the browser for routine listening.
2. **Terminal-native.** Sub-second startup, low CPU, works over SSH, vim-friendly keys, themable, no mouse required (but mouse supported).
3. **The visualiser is the identity.** Not a decorative afterthought — it is the thing on screen where a cover image would be, and it must be beautiful, accurate, and cheap.

### Non-goals

- Video playback (YTM music videos play as audio-only).
- Being a general YouTube client.
- Downloading/DRM circumvention as a feature. Caching is for playback smoothness and offline convenience of content the user's account has access to; see §5.
- Podcasts and YouTube Music uploads in v1 (candidates for v2).

---

## 2. Prior art & differentiation

| Project | Language | What it does | Gap we exploit |
|---|---|---|---|
| [ytermusic](https://github.com/ccgauche/ytermusic) | Rust | Fast, minimal, privacy-focused player over your liked/playlists | Minimal by design: no Home feed, no ratings, no visualiser |
| [youtui](https://crates.io/crates/youtui) | Rust | Artist→Album discovery workflow, cmus-inspired | Discovery-first; limited "official player" parity, no visualiser |
| [ytui-music](https://github.com/sudipghimire533/ytui-music) | Rust | Lightweight YouTube-as-music client | Generic YouTube, not YTM library/ratings semantics |
| [ytmusic-tui](https://lib.rs/crates/ytmusic-tui) / ytm-player | Rust | Closest in scope: library, likes, MPRIS, synced lyrics, vim keys | Closest competitor. Differentiation = visualiser quality + Home/Explore parity + polish |
| cava | C | Best-in-class terminal spectrum | Not a player — we internalise its ideas |
| spotify-player | Rust | The bar for TUI music UX (incl. image support) | Different backend; borrow UX patterns |

**Differentiation statement:** every existing TUI YTM client treats the "now playing" pane as a text block. `ytmtui` treats it as a canvas. Combined with full Home/Explore/radio parity, that is the product.

---

## 3. Users & scenarios

- **P1 — Terminal-resident developer (primary, = you).** Lives in a tiling WM. Wants music without a browser tab eating 400 MB. Cares about keybindings, theming, and that likes sync back to the real account so mobile/car playback stays consistent.
- **P2 — Remote/SSH listener.** Runs on a home server or over SSH into a workstation; needs the audio sink to be configurable and the UI to degrade gracefully on a 80×24 dumb terminal.
- **P3 — Low-power/retro enthusiast.** Old laptop, Raspberry Pi. Needs a CPU budget and a "cheap visualiser" mode.

**Key scenarios**
- S1: Launch → last session restored → `Space` resumes the queue.
- S2: `/` → type "aphex twin" → tab to Songs → `Enter` plays, radio auto-continues after the track.
- S3: A track plays; user hits `+` to thumbs-up → the like appears in the account's Liked Songs on the phone.
- S4: User hits `d` on a track that keeps showing up → thumbs-down → skipped and demoted in radio.
- S5: User resizes the terminal from 200 cols to 60 — layout reflows, spectrum re-bins band count, nothing panics.
- S6: Network drops mid-track — playback continues from the prefetched buffer, a status indicator warns, and recovery is automatic.

---

## 4. Domain analysis — how YouTube Music actually works

This section drives most of the hard requirements. YouTube Music has **no official public API**; everything below is the reverse-engineered *InnerTube* API that the web client itself uses.

### 4.1 InnerTube

- Endpoints under `music.youtube.com/youtubei/v1/*` (`browse`, `search`, `next`, `player`, `like/like`, `like/dislike`, `like/removelike`, `browse/edit_playlist`, `feedback`).
- Requests carry a `context` object identifying a client (`WEB_REMIX` for YTM) and a client version, which must be kept current or responses shift shape.
- Responses are deeply nested renderer trees. **They change without notice.** Parsing must be defensive: unknown renderer → skip and log, never panic.

### 4.2 Authentication

Two workable modes:

| Mode | How | Gives you |
|---|---|---|
| **Cookie / SAPISIDHASH** (required for parity) | User supplies browser cookies incl. `__Secure-3PAPISID`; each request signs an `Authorization: SAPISIDHASH <ts>_<sha1(ts + sapisid + origin)>` header | Home feed, library, playlists, history, **likes/dislikes**, playlist edits |
| **OAuth (TV client)** | Device-code flow | Streams and some reads; **does not** give the YTM user-data surface |
| Anonymous | No auth | Search, charts, public playlists, streams. Useful as a degraded mode |

**Implication:** the account features the user cares about (like/dislike, library, Home) require cookie auth. This must be a first-class, well-documented, secure onboarding flow — not an afterthought.

### 4.3 Streams and the PO-token problem

- `player` returns adaptive formats. Audio-only candidates: **itag 251** (Opus ~160 kbps, WebM), **itag 141/140** (AAC-LC 256/128 kbps, MP4), plus lower bitrates 249/250/139.
- Since Aug 2024 **web-based clients must attach a PO Token ("proof of origin")** or stream URLs return HTTP 403. In practice this means running a small BotGuard/JS-VM helper. `rustypipe` solves this by shelling out to a `rustypipe-botguard` binary found on `PATH`.
- Some URLs are additionally throttled unless an `n`-parameter is deobfuscated by executing player JS.

**Implication (major):** stream acquisition is the single most fragile subsystem. It must be isolated behind a trait so the strategy can be swapped (native InnerTube → helper binary → `yt-dlp` fallback) without touching the player.

> **Revised (see companion doc §5):** the `yt-dlp` fallback resolver is promoted to **v1/M0 scope**, not "later." It is the cheapest insurance in the project.
>
> **M0 measured:** PO tokens are **not currently required** on the yt-dlp path — it routes via a `VISIONOS` client that is exempt, and resolves anonymously with no cookies and no BotGuard helper. Treat this as a moving target Google can close at any time, not a permanent result.

### 4.4 Codec reality check ⚠️

Symphonia (the pure-Rust decode stack behind rodio) currently lists **Opus as "in progress" / incomplete**, while AAC-LC, Vorbis, MP3 and FLAC are solid.

Therefore:

- **Default path (v1):** select **itag 140 (AAC-LC 128 kbps, MP4/M4A)** and decode with Symphonia's `aac` + `isomp4` features. Pure Rust, no external deps.
- **Optional quality path:** feature flag `opus` binding native libopus (`audiopus`/`opus` crate) with Symphonia's MKV/WebM demuxer, unlocking itag 251 (~160 kbps Opus, audibly better).
- **Escape hatch:** feature flag `ffmpeg-decode` / external `mpv --no-video` backend for users who already have it and want maximum robustness.

This is a **requirement, not a detail** — picking Opus by default would produce silence.

> **Revised (see companion doc §5):** the recommended default is now an **ffmpeg-piped PCM decoder**, which eliminates this risk entirely and unlocks itag 251 Opus. The symphonia AAC path is retained as the zero-dependency fallback so `cargo install` works with no system deps.

### 4.5 Ratings semantics (important nuance)

Mimicking the official player means getting these three *distinct* actions right, because users conflate them:

1. **Thumbs up (`rate_song` → `like/like`)** — adds to the *Liked Songs* auto-playlist and biases radio. Does **not** add to Library.
2. **Add to Library (`edit_song_library_status` with feedback tokens)** — different operation, different token, different UI affordance.
3. **Thumbs down (`like/dislike`)** — suppresses the track in radio/autoplay; the official client also skips the track.

Feedback tokens for #2 are per-item and only present in *authenticated* responses, so the parser must retain them alongside each track.

### 4.6 Library landscape (Rust)

| Need | Candidate | Assessment |
|---|---|---|
| Reads (search, home, artist, album, playlist, radio, lyrics, charts, saved items, history) | [`rustypipe`](https://docs.rs/rustypipe/) 0.11.x | Mature, actively maintained, handles PO tokens via botguard helper, cookie auth. **However: the API is read-only.** |
| Writes (rate song, like/unlike, library status, playlist edit) | [`ytmusicapi`](https://docs.rs/ytmusicapi) (Rust) 0.5.0 | Covers playlists, library, liked songs, **rating** — but no search. Single maintainer, pre-1.0. |
| Fallback / last resort | own thin InnerTube client, or shelling to Python `ytmusicapi` / `yt-dlp` | Full control, full maintenance burden |

**Recommended composition:** `rustypipe` for all reads + `ytmusicapi` (or a ~400-line in-house `innertube-write` module) for the handful of mutations. Both behind our own `MusicBackend` trait so either can be replaced.

---

## 5. Legal & ethical constraints

- This is an **unofficial** client using private APIs; it may break at any time and arguably sits outside YouTube's ToS. The README must say so plainly.
- **Do not** implement or advertise permanent audio ripping. Cache = bounded, encrypted-at-rest optional, evictable, tied to playback.
- **Respect the account's Premium status.** If the account is not Premium, YouTube serves ads/limits; the client must not attempt ad-stripping.
- **Credentials never leave the machine.** Cookies stored 0600, ideally in the OS keyring; never logged, never in crash reports.
- Send a plausible, honest client identity; do not attempt to defeat rate limiting or bot detection beyond what is required for the account's own playback.

---

## 6. Functional requirements

Priority: **M** = must (v1.0), **S** = should, **C** = could (v2), **W** = won't (this release).

### 6.1 Authentication & account

| ID | Requirement | Pri |
|---|---|---|
| FR-A1 | Cookie-based login: guided flow (`ytmtui auth`) accepting a pasted cookie header or a browser-cookie import, validated by a live "whoami" call | M |
| FR-A2 | ✅ Credentials stored in OS keyring when available, else `~/.config/ytmtui/auth.json` mode 0600 | M |
| FR-A3 | Anonymous mode: search + charts + public playlists work with no login; account-gated UI is visibly disabled, not hidden | M |
| FR-A4 | ✅ Detect expired/invalid session and prompt for re-auth without losing playback state | M |
| FR-A5 | Multiple profiles/accounts, switchable at runtime | C |

### 6.2 Browse & discover (Home / Explore parity)

| ID | Requirement | Pri |
|---|---|---|
| FR-B1 | ✅ **Home** feed: the account's real shelves (Listen again, Quick picks, Mixed for you, recommended albums/playlists), horizontally scrollable carousels | M |
| FR-B2 | ✅ **Explore**: New releases, Charts (with country selector), Moods & genres | S |
| FR-B3 | ✅ Entity pages: **Artist** (top songs, albums, singles, related, subscribe), **Album** (tracklist, year, play/shuffle), **Playlist** (tracks, description, owner) | M |
| FR-B4 | ✅ Infinite scroll / continuation loading with a loading indicator | M |
| FR-B5 | ✅ "More from this artist / go to album / go to artist" context navigation from any track | M |
| FR-B6 | ✅ Back/forward navigation stack (`Esc` / `Ctrl-o`, `Ctrl-i`) | S |

### 6.3 Search

| ID | Requirement | Pri |
|---|---|---|
| FR-S1 | ✅ Global search with result tabs: Top result, Songs, Videos, Albums, Artists, Playlists, Community playlists | M |
| FR-S2 | ✅ Search-as-you-type suggestions, debounced (~200 ms), cancellable in-flight requests | S |
| FR-S3 | ✅ Search history, recallable with ↑ in the search bar | S |
| FR-S4 | Filter/scope search within the current library view | C |

### 6.4 Library

| ID | Requirement | Pri |
|---|---|---|
| FR-L1 | ✅ Library sections: Playlists, Songs (liked + library), Albums, Artists, Subscriptions | M |
| FR-L2 | Liked Songs playlist, always present and playable | M |
| FR-L3 | ✅ Recently played / History view | S |
| FR-L4 | ✅ Create / rename / delete playlist; add & remove tracks; reorder | S |
| FR-L5 | Uploads (personal library) | W |

### 6.5 Playback

| ID | Requirement | Pri |
|---|---|---|
| FR-P1 | Play / pause / stop; next / previous (previous restarts track if >3 s elapsed, like the official player) | M |
| FR-P2 | Seek: relative (±5 s / ±30 s) and absolute (click/drag on progress bar, or `g` + timestamp) | M |
| FR-P3 | Volume 0–150 % with a soft limiter above 100 %, plus mute toggle | M |
| FR-P4 | Repeat modes: off → all → one; Shuffle on/off (Fisher-Yates over the queue, preserving the current track) | M |
| FR-P5 | ✅ **Gapless playback** between consecutive tracks (prefetch + decode-ahead of the next item) | S |
| FR-P6 | Crossfade, configurable 0–12 s | C |
| FR-P7 | Playback speed 0.5×–2.0× with pitch preserved | C |
| FR-P8 | ✅ Audio quality selector: Low / Normal / High / Auto-by-bandwidth, mapped to itags | S |
| FR-P9 | ✅ Output device selection (list ALSA/Pulse/Pipewire/CoreAudio sinks); survives device disappearance | S |
| FR-P10 | Robust buffering: prefetch window, stall detection, automatic retry with exponential backoff, resume at the same offset | M |
| FR-P11 | Normalisation / ReplayGain-style loudness levelling using the stream's loudnessDb | C |
| **FR-P12** | **Resolve stream URLs ahead of need**: resolve the next queue item during the current track, cache resolved URLs until their `expire`, spinner on unavoidable cold resolve. Measured yt-dlp cold resolve = **3.4 s**, far too slow to sit in the interactive path | **M** |

### 6.6 Queue, radio & autoplay

| ID | Requirement | Pri |
|---|---|---|
| FR-Q1 | Visible, editable queue: reorder, remove, jump-to, clear | M |
| FR-Q2 | "Play next" vs "Add to queue" as distinct actions | M |
| FR-Q3 | ✅ **Autoplay/radio**: when the queue drains, continue with a station seeded from the last track — matching official behaviour, toggleable with `A` | M |
| FR-Q4 | ✅ "Start radio" from the selected song (`R`); artist/album/playlist seeds pending their entity pages | M |
| FR-Q5 | ✅ Queue persisted across restarts (track + position) | S |

### 6.7 Ratings & social — *the like/dislike surface*

| ID | Requirement | Pri |
|---|---|---|
| FR-R1 | **Thumbs up** on the now-playing track and on any track in a list; optimistic UI update, rollback + toast on failure | M |
| FR-R2 | **Thumbs down**; by default also skips the track (configurable), mirroring the official player | M |
| FR-R3 | Toggling off a rating (press again → `like/removelike`) | M |
| FR-R4 | ✅ **Add to / remove from Library**, kept visually and semantically distinct from thumbs-up (§4.5) | M |
| FR-R5 | ✅ Add track to a playlist via a picker | S |
| FR-R6 | ✅ Subscribe / unsubscribe to an artist | S |
| FR-R7 | ✅ Rating state is fetched with each track so the UI shows the *true* current state, not a guess | M |
| FR-R8 | ✅ Share: copy the track/album URL to the clipboard | S |
| FR-R9 | Write operations implemented **in-house** (`innertube-write`: SAPISIDHASH + like/dislike/removelike/library endpoints) rather than via a pre-1.0 third-party crate — see companion doc §8 | M |

### 6.8 Lyrics

| ID | Requirement | Pri |
|---|---|---|
| FR-Y1 | ✅ Plain lyrics panel via `music_lyrics` | S |
| FR-Y2 | Time-synced lyrics with the active line highlighted and auto-scroll, when a synced source is available (e.g. LRCLIB as a supplementary provider) | C |
| FR-Y3 | ✅ Graceful "no lyrics available" state | S |

### 6.9 Visualisation — *the differentiator* (detailed in §7)

| ID | Requirement | Pri |
|---|---|---|
| FR-V1 | Real-time spectrum analyser fed from the **actual decoded PCM** being sent to the sound card, not a simulation | M |
| FR-V2 | ≥4 render styles: **Bars**, **Mirrored bars**, **Oscilloscope/waveform**, **Level/VU meters** | M |
| FR-V3 | Perceptual frequency binning (log/Bark scale), band count derived from available width | M |
| FR-V4 | Configurable smoothing, decay/falloff, and **peak-hold caps** | M |
| FR-V5 | Colour mapping: gradient by amplitude and/or by frequency; theme-driven; truecolor with 256-colour and 16-colour fallbacks | M |
| FR-V6 | High-resolution glyph rendering: **braille / octant / sextant / half-block**, auto-selected from detected font capability, user-overridable | M |
| FR-V7 | Visualiser can be resized, moved to full-screen (`z`), or disabled entirely (CPU saving) | M |
| FR-V8 | Audio/visual alignment: visual frames delayed to compensate for output-device latency so bars match what is *heard* | S |
| FR-V9 | Beat/onset detection driving accent effects (colour pulse, border flash) | C |
| FR-V10 | Optional album art via Kitty/iTerm2/Sixel graphics protocols as an *alternative* pane, for users who want it | C |

### 6.10 Interface & interaction

| ID | Requirement | Pri |
|---|---|---|
| FR-U1 | Persistent layout: sidebar (nav) + main content + now-playing bar + visualiser pane | M |
| FR-U2 | Responsive down to 80×24; progressive disclosure as width grows; never panics on tiny terminals | M |
| FR-U3 | ✅ Vim-style keys by default, with arrows/emacs alternates; **fully remappable** via config | M |
| FR-U4 | Built-in help overlay (`?`) generated from the live keymap | M |
| FR-U5 | ✅ Command palette (`:`) for actions without a binding | S |
| FR-U6 | ✅ Mouse support: click to focus/select, click-seek on the progress bar, scroll wheel | S |
| FR-U7 | ✅ Themes: ship several (incl. YTM-red dark, gruvbox, catppuccin, monochrome); user TOML themes; auto-detect terminal background | S |
| FR-U8 | Non-blocking toast/notification area for errors and confirmations | M |
| FR-U9 | Loading skeletons/spinners — **the UI never blocks on network I/O** | M |

### 6.11 System integration

| ID | Requirement | Pri |
|---|---|---|
| FR-I1 | ✅ **MPRIS2** D-Bus interface on Linux (playerctl, media keys, desktop widgets) | S |
| FR-I2 | Media-key handling on macOS/Windows | C |
| FR-I3 | Desktop notification on track change (configurable) | C |
| FR-I4 | Discord Rich Presence / scrobbling to Last.fm or ListenBrainz | C |
| FR-I5 | CLI subcommands for scripting: `ytmtui play <query>`, `next`, `status --json` (IPC to a running instance) | C |

### 6.12 Configuration, cache & diagnostics

| ID | Requirement | Pri |
|---|---|---|
| FR-C1 | ✅ TOML config at `$XDG_CONFIG_HOME/ytmtui/config.toml`, hot-reload on save | S |
| FR-C2 | Bounded on-disk cache (default 1 GiB, LRU) for audio segments + metadata + a small thumbnail cache | M |
| FR-C3 | ✅ Metadata cache with TTLs (search 5 min, artist/album 24 h, home 15 min) and a manual refresh key | S |
| FR-C4 | ✅ `--log-level`, log to file, and a `ytmtui doctor` command reporting backend/codec/PO-token/terminal-capability status | S |
| FR-C5 | ✅ Session restore: last view, queue, track, position | S |

### 6.13 Account safety (see R11)

| ID | Requirement | Pri |
|---|---|---|
| FR-N1 | Traffic must resemble a music player, not a scraper: one active stream plus at most one next-track prefetch; no bulk or speculative fetching of whole playlists | M |
| FR-N2 | Global rate limiting and exponential backoff; never retry-loop a failing request | M |
| FR-N3 | No download/export/ripping feature, ever | M |
| FR-N4 | README states the account-restriction risk plainly, before any auth instructions | M |
| FR-N5 | `--read-only` mode that performs no writes and (optionally) no authentication at all | S |

---

## 7. Deep dive — the spectrum visualiser

This is the feature that justifies the project, so it gets real specification.

### 7.1 Signal path

```
HTTP stream ─► demux ─► decode ─► PCM f32 frames
                                     │
                          ┌──────────┴──────────┐
                          ▼                     ▼
                   [ Tap: SPSC ring ]     rodio Sink ─► cpal ─► device
                          │
                          ▼
                 Analyser thread (@60 Hz)
                  window ─► FFT ─► magnitude ─► band bins
                  ─► perceptual weighting ─► smoothing ─► peaks
                          │
                          ▼
                 ArcSwap<SpectrumFrame> ──read──► UI render loop
```

**Tap design.** Wrap the decoder in a custom `rodio::Source` that copies each sample into a lock-free SPSC ring buffer as it passes through to the mixer. This is exact (it sees post-volume, post-resample audio), adds no allocation on the audio path, and cannot block the audio thread — the writer drops samples rather than waiting. *Never* allocate, lock, or log in this path.

**Latency compensation (FR-V8).** The tap sees samples ~one device buffer (5–100 ms) *before* they are audible. Delay the analyser's read pointer by the measured output latency so the bars line up with what the ear hears. Expose `visualizer.latency_offset_ms` for manual trim.

### 7.2 DSP parameters (defaults)

| Parameter | Default | Notes |
|---|---|---|
| FFT size | 2048 samples @44.1 kHz (≈46 ms, 21.5 Hz/bin) | 4096 in "high detail" mode; 1024 in "low CPU" |
| Hop / frame rate | 60 fps target, decoupled from FFT size | Renders at terminal refresh, not per-FFT |
| Window | Hann | Blackman-Harris option for cleaner peaks |
| Channels | Mono downmix by default | Stereo mode: left/right split panes or mirrored |
| Frequency range | 30 Hz – 16 kHz | Above 16 kHz is empty in lossy audio and looks dead |
| Band mapping | Logarithmic / Bark-like, `bands = clamp(width_cells * cells_per_band, 8, 128)` | Musically meaningful; linear bins cram all music into the left 10 % |
| Weighting | A-weighting-ish tilt + per-band gain curve | Otherwise bass dwarfs everything |
| Scale | dB, floor **−45 dB** (measured, M0 §6), ceiling from a slow rolling max (AGC) | A 60 dB window bunched the median band at 0.59 and read as a solid wall |
| Gamma | **1.4** | Expands the quiet end; tuned against measured percentiles |
| HF tilt | **+1.0 dB/octave** | Compensates music's natural roll-off so the top stays legible |
| Attack / release | attack ≈ instant, release ≈ 8 dB per 100 ms | Classic analyser feel |
| Inter-band smoothing | Light 3-tap blur | Removes comb artefacts from sparse bins |
| Peak hold | 700 ms hold, then fall at 12 dB/s | Small cap glyph above each bar |
| Silence handling | Below −55 dB → decay to zero and idle at ~5 fps | Saves CPU during pauses/gaps |

Crates: `rustfft` (or `realfft` for a ~2× win on real input) + `apodize`/hand-rolled window. `spectrum-analyzer` is a viable higher-level shortcut for the MVP.

### 7.3 Rendering in a terminal

Ratatui 0.30 ships several sub-cell markers on `Canvas`, which is exactly the tool for this:

| Marker | Sub-cell resolution | Use |
|---|---|---|
| `Braille` | 2×4 | Oscilloscope traces, fine bars |
| `Octant` | 2×4, **densely packed & regular** | **Preferred for bars** — no braille inter-dot gaps |
| `Sextant` | 2×3 | Middle ground, wider font support |
| `HalfBlock` | 1×2 | Maximum compatibility |
| `Bar`/`Block` | 1×1 | 16-colour / dumb terminals |

**Auto-detect** with a font-capability probe and a config override (`visualizer.marker = "auto" | "octant" | "braille" | "sextant" | "halfblock" | "block"`), because octant/sextant glyphs are recent Unicode and not in every font.

Vertical resolution trick: combine the marker's sub-cell rows with **partial block glyphs** (`▁▂▃▄▅▆▇█`) for a ~8× vertical resolution per cell in bar mode.

### 7.4 Render styles (FR-V2)

```
Bars                    Mirrored                Oscilloscope           VU
     ▃                       ▃                                          L ███████▌  -6dB
   ▅ █ ▆                   ▅ █ ▆              ╭─╮   ╭╮                  R ██████▊   -7dB
 ▂ █ █ █ ▄               ▂ █ █ █ ▄          ──╯ ╰─╮╭╯╰──╮╭──
 █ █ █ █ █               █ █ █ █ █                ╰╯     ╰╯             ▁▂▃▅▆▇ peak history
─────────────            ▀ ▀ ▀ ▀ ▀
                         █ █ █ █ █
                           ▀ █ ▀
```

Plus a **Spectrogram / waterfall** mode (scrolling heat-map of the last N seconds) as a `C`-priority extra — cheap to add once binning exists, and visually striking.

### 7.5 Colour

- **By amplitude** (default): theme gradient low→high, e.g. `#1db954`-style green → amber → the YTM red at clipping.
- **By frequency**: hue mapped across the spectrum (bass warm → treble cool). Reads beautifully in truecolor.
- **Static**: single accent colour for 16-colour terminals and minimal themes.
- Peak caps drawn in a contrasting/dimmed colour.
- Degradation ladder: truecolor → 256 → 16 → monochrome (intensity via glyph density only). Detect via `COLORTERM`/terminfo, allow override.

### 7.6 Performance budget

| Metric | Target |
|---|---|
| Visualiser CPU (bars, 60 fps, 2048-pt FFT, 1 core) | **< 3 %** on a modern laptop core |
| Whole app while playing, visualiser on | < 6 % |
| Whole app while playing, visualiser off | < 1.5 % |
| Idle (paused, no input) | ~0 % — event-driven, no busy render loop |
| Frame rate adapts to terminal size and a `visualizer.max_fps` setting (default 60, cap 144, floor 15) — **load-bearing**: M0 measured ~119 fps uncapped, roughly doubling visualiser CPU for no visible gain | |

The analyser runs on its own thread and publishes an immutable `SpectrumFrame` via `ArcSwap`; the UI reads the latest frame at *its* pace. Dropping analyser frames is always acceptable. Rendering must never wait on DSP, and DSP must never wait on rendering.

---

## 8. Non-functional requirements

| ID | Requirement |
|---|---|
| NFR-1 | **Startup:** first frame < 150 ms; UI interactive before any network call completes |
| NFR-2 | **Responsiveness:** every keypress acknowledged within one frame (< 16 ms); zero blocking I/O on the UI thread |
| NFR-3 | **Memory:** < 120 MB RSS steady-state with visualiser on and a 500-track queue |
| NFR-4 | **Audio integrity:** no underruns/glitches under normal load; audio thread is realtime-safe (no alloc, no locks, no syscalls) |
| NFR-5 | **Resilience:** no panics reach the user. Global panic hook restores the terminal, writes a report, exits cleanly. Backend parse failures degrade to a visible empty/error state |
| NFR-6 | **Terminal compatibility:** alacritty, kitty, wezterm, foot, ghostty, xterm, tmux, screen, Windows Terminal; correct behaviour on resize, suspend (`Ctrl-Z`) and resume |
| NFR-7 | **Portability:** Linux (primary, incl. PipeWire/Pulse/ALSA), macOS, Windows; single static-ish binary, no runtime Python |
| NFR-8 | **Accessibility:** no colour-only information (glyph/shape carries meaning too); high-contrast theme; screen-reader-friendly text mode that suppresses the visualiser |
| NFR-9 | **Security:** credentials 0600 or keyring; redacted logs; TLS verification never disabled; no telemetry |
| NFR-10 | **Maintainability:** backend behind traits with recorded-fixture tests, so an InnerTube shape change is a contained fix |
| NFR-11 | **Observability:** `tracing` spans, `--log-file`, `ytmtui doctor` self-check |
| NFR-12 | **Build:** stable Rust (MSRV ≥ 1.85 per Symphonia), `cargo build` works with no system deps in the default feature set |
| **NFR-13** | **No subprocess may discard its stderr.** M0 proved the failure mode: a nulled ffmpeg stderr turned a hard error into a healthy-looking UI with an empty spectrum and no diagnostic anywhere. Drain it on a thread and surface it |

---

## 9. Proposed architecture

### 9.1 Crate layout (Cargo workspace)

```
ytmtui/
├── crates/
│   ├── ytm-core/        # domain types: Track, Album, Artist, Playlist, Rating, Queue
│   ├── ytm-api/         # MusicBackend trait; rustypipe reads + innertube writes; auth; caching
│   ├── ytm-player/      # stream fetch, decode, rodio sink, queue engine, PCM tap
│   ├── ytm-viz/         # FFT, banding, smoothing, SpectrumFrame  (pure, no I/O — unit-testable)
│   ├── ytm-tui/         # ratatui views, widgets (incl. SpectrumWidget), theming, keymap
│   └── ytm-ipc/         # MPRIS + CLI control    [later]
└── src/main.rs          # wiring, config, panic hook
```

`ytm-viz` being I/O-free means the visualiser can be developed and benchmarked against WAV files long before any YouTube plumbing works — **de-risking the hardest-to-tune feature first.**

### 9.2 Concurrency model

```
┌──────────────┐  Action   ┌──────────────┐  Command  ┌───────────────┐
│  UI thread   │──────────►│  App/State   │──────────►│ Player engine │
│ (render+keys)│◄──────────│   (tokio)    │◄──────────│  (audio thr.) │
└──────┬───────┘  Event    └──────┬───────┘  Event    └───────┬───────┘
       │                          │                           │
       │ ArcSwap<SpectrumFrame>   │ async tasks               │ SPSC ring
       │                          ▼                           ▼
       │                   ┌──────────────┐           ┌───────────────┐
       └───────────────────│  Analyser    │◄──────────│   PCM tap     │
                           └──────────────┘           └───────────────┘
```

- **Message-passing, single source of truth.** `AppState` is owned by one task; the UI renders a snapshot. No `Arc<Mutex<AppState>>` shared everywhere.
- The UI loop selects over: terminal events, app events, a frame tick (visualiser fps), and a slow tick (progress/1 s).
- Network work is `tokio` tasks that emit events; every request is cancellable (search-as-you-type depends on it).
- The audio callback owns nothing that can block.

### 9.3 Key abstractions

```rust
trait MusicBackend {                       // swappable: rustypipe / mock / fixtures
    async fn search(&self, q: &str, filter: SearchFilter) -> Result<SearchResults>;
    async fn home(&self) -> Result<Vec<Shelf>>;
    async fn track_details(&self, id: &VideoId) -> Result<TrackDetails>; // incl. rating + feedback tokens
    async fn radio(&self, seed: RadioSeed) -> Result<Vec<Track>>;
    async fn rate(&self, id: &VideoId, rating: Rating) -> Result<()>;   // Like | Dislike | Indifferent
    async fn set_library(&self, tok: &FeedbackToken, in_lib: bool) -> Result<()>;
    /* … */
}

trait StreamResolver {                     // isolates the PO-token fragility (§4.3)
    async fn resolve(&self, id: &VideoId, pref: QualityPref) -> Result<AudioStream>;
}

struct SpectrumFrame { bands: Box<[f32]>, peaks: Box<[f32]>, rms: f32, seq: u64 }
```

### 9.4 Tech stack

| Concern | Choice | Why / risk |
|---|---|---|
| TUI | **ratatui 0.30** + crossterm | Octant/sextant markers, `ratatui::run()`, modular crates |
| Async | tokio | Required by rustypipe/reqwest |
| YTM reads | rustypipe 0.11 | Most complete & maintained; GPL-3.0 → **licence choice is forced to GPL-3.0** ⚠️ |
| YTM writes | ytmusicapi (Rust) 0.5 or in-house | Read-only gap in rustypipe |
| PO tokens | rustypipe-botguard helper | External binary dependency; document + `doctor` check. Note: PO tokens no longer guarantee bypass |
| Stream resolution | `StreamResolver` trait: rustypipe native **+ yt-dlp fallback from day one** | yt-dlp absorbs Google-side churn faster than we ever could |
| Audio out | rodio 0.22 (cpal) | Simple sink, gapless option |
| Decode | **ffmpeg pipe (recommended default)**; symphonia 0.6 (`aac`,`isomp4`) as zero-dep fallback | Revised — removes the Opus gap entirely (companion §5) |
| FFT | realfft / rustfft | Fast, no_std-friendly |
| HTTP | reqwest (rustls) | Range requests for seeking |
| Config | serde + toml + directories | |
| Errors/logging | thiserror + anyhow + tracing | |
| Storage | sled or rusqlite for cache index | rusqlite if history/stats grow |

⚠️ **Licence note:** rustypipe is GPL-3.0. Linking it makes `ytmtui` GPL-3.0. Decide deliberately (see §12 Q6).

---

## 10. Interface design

### 10.1 Main layout (wide terminal)

```
┌ ytmtui ─────────────────────────────────────────────────── ⏻ petr ──┐
│ ⌕ Search…                                                            │
├──────────────┬───────────────────────────────────────────────────────┤
│  ▸ Home      │  Quick picks                                          │
│    Explore   │  ┌────────────┐┌────────────┐┌────────────┐           │
│    Library   │  │ Windowlick ││ Xtal 23    ││ Ageispolis │  ▸        │
│    ───────   │  │ Aphex Twin ││ Aphex Twin ││ Aphex Twin │           │
│    Liked     │  └────────────┘└────────────┘└────────────┘           │
│    History   │                                                        │
│    ───────   │  Listen again                                          │
│    Playlists │  ┌────────────┐┌────────────┐┌────────────┐           │
│      Chill   │  │ …          ││ …          ││ …          │  ▸        │
│      Focus   │  └────────────┘└────────────┘└────────────┘           │
├──────────────┴───────────────────────┬───────────────────────────────┤
│        ▂▄ █ ▆ ▃ █ ▅ ▂ ▇ █ ▄ ▂        │ Queue                    (12) │
│      ▃ ██ █ █ █ █ █ █ █ █ █ █ ▃      │  1 ▸ Xtal — Aphex Twin        │
│    ▅ █ ██ █ █ █ █ █ █ █ █ █ █ █ ▄    │  2   Tha — Aphex Twin         │
│  ▂ █ █ ██ █ █ █ █ █ █ █ █ █ █ █ █ ▃  │  3   Ptolemy — Aphex Twin     │
├───────────────────────────────────────┴───────────────────────────────┤
│ ▶  Xtal · Aphex Twin · Selected Ambient Works 85-92    👍 ♥  🔀 🔁    │
│ ├──────────●───────────────────────────────────┤  01:47 / 04:51  🔊72 │
└───────────────────────────────────────────────────────────────────────┘
```

### 10.2 Narrow terminal (80×24)

Sidebar collapses to icons or hides behind `Tab`; carousels become vertical lists; visualiser shrinks to a single bar row above the now-playing line; queue moves to an overlay (`q`).

### 10.3 Keymap (default)

| Key | Action | | Key | Action |
|---|---|---|---|---|
| `Space` | Play / pause | | `+` / `l` | **Thumbs up** (toggle) |
| `n` / `p` | Next / previous | | `-` / `d` | **Thumbs down** (toggle) |
| `→` `←` | Seek ±5 s | | `a` | Add to / remove from **Library** |
| `⇧→` `⇧←` | Seek ±30 s | | `A` | Add to playlist… |
| `↑` `↓` `j` `k` | Navigate | | `s` | Shuffle toggle |
| `Enter` | Play selection | | `r` | Repeat cycle |
| `o` | Play next | | `R` | Start radio from selection |
| `e` | Add to queue end | | `y` | Copy link |
| `/` | Search | | `v` | Cycle visualiser style |
| `:` | Command palette | | `z` | Full-screen visualiser |
| `Tab` | Cycle pane focus | | `V` | Toggle visualiser off/on |
| `q` | Queue overlay | | `L` | Lyrics panel |
| `Esc` | Back | | `?` | Help |
| `9` / `0` | Volume −/+ | | `Ctrl-c` / `Q` | Quit |

All remappable; the help overlay is generated from the active map so it never drifts.

---

## 11. Risks & mitigations

| # | Risk | Impact | Likelihood | Mitigation |
|---|---|---|---|---|
| R1 | InnerTube response shapes change | Features break silently | **High** | Defensive parsers; fixture-based regression tests; per-feature degradation, never a crash; pin & track upstream crate releases |
| R2 | PO-token / bot-detection tightening blocks streams | Degraded, recoverable | Medium | **Revised down** by shipping the yt-dlp resolver in v1: a break becomes a `pip install -U yt-dlp` away from working, not a dead product. `doctor` diagnoses |
| R3 | Opus decode unavailable in pure Rust | — | — | **Eliminated** by the ffmpeg decode path (companion §5). Symphonia AAC retained as fallback; smoke test per codec |
| R4 | Visualiser eats CPU / stutters the UI | Core feature feels bad | Medium | Separate thread, ArcSwap, fps cap, adaptive quality, benchmark suite in `ytm-viz` |
| R5 | Audio thread blocked → glitches | Audible defects | Medium | Lock-free tap, no alloc in callback, ring-buffer overwrite semantics |
| R6 | Cookie auth is a bad UX / people leak cookies | Adoption + safety | Medium | Guided `auth` flow, keyring, explicit security notes, anonymous mode that actually works |
| R7 | GPL-3.0 propagation from rustypipe | Licensing surprise | **Void as of M1** | M1 uses an in-house InnerTube client, not rustypipe, so there is currently no GPL dependency and the licence is a free choice. Adopting rustypipe for richer reads in M2 would reintroduce this |
| R8 | Unicode markers unsupported in user's font | Ugly visualiser | Medium | Capability probe + ladder down to half-block/ASCII; setting to force |
| R9 | Rate limiting / account flags from aggressive polling | Account risk | Low-Medium | Conservative caching, no polling loops, backoff, single client identity |
| R10 | Scope creep (this doc is big) | Never ships | **High** | Strict M/S/C gating; M0–M2 below is a complete usable product |
| **R11** | **Account restriction/ban from third-party authenticated access.** OAuth no longer works; cookies are the only route and carry documented ban risk. A throwaway account is not an option — likes must land on the real account | **Loss of the user's Google account** | Low-Medium | **Highest-severity risk in the project, and no technology choice affects it.** Mitigations are behavioural: FR-N1–N5 (player-shaped traffic, aggressive caching, rate limiting, no download feature, honest README, read-only mode) |

---

## 12. Open questions — decisions needed

| # | Question | Recommendation |
|---|---|---|
| Q1 | Is the target account **YouTube Music Premium**? | Affects ad handling and available bitrates. Assume Premium; degrade gracefully if not |
| Q2 | Audio quality default: pure-Rust AAC 128k vs. libopus 160k? | Ship AAC default, `--features opus` documented for audiophiles |
| Q3 | Which is the "hero" visualiser style at first launch? | Mirrored bars, octant marker, amplitude gradient |
| Q4 | Stereo or mono spectrum by default? | Mono (wider bands, clearer); stereo split as a mode |
| Q5 | Album art at all (Kitty/Sixel) as an alternate pane? | Yes, but `C` priority — the spectrum stays the default |
| Q6 | Licence: accept GPL-3.0 (rustypipe) or write an MIT-compatible InnerTube client? | Accept GPL-3.0 for v1; revisit if distribution matters |
| Q7 | Thumbs-down auto-skip on by default? | Yes — matches the official player; make it configurable |
| Q8 | MPRIS in v1 or v2? | v1.1 — cheap and disproportionately loved on Linux |
| Q9 | Offline caching of liked songs for airplane use? | Out of scope for v1 (see §5) |
| Q10 | Minimum supported terminal size? | 60×16 renders something usable; below that, a "resize me" message |

---

## 13. Roadmap

| Milestone | Contents | Acceptance criteria |
|---|---|---|
| **M0 — Spike (1 wk)** | Prove the three risky things *separately*: (a) authenticated InnerTube read + a successful like via write path; (b) resolve a stream URL via **both** resolvers (native + yt-dlp) and decode via **both** paths (ffmpeg + symphonia); (c) `ytm-viz` rendering a spectrum from a local WAV in ratatui | Three throwaway binaries, each demonstrably working — tested against the *fallbacks*, not just the happy path. **Go/no-go on R2, R3 & R11 here.** |
| **M1 — Skeleton player** ✅ | Workspace, search → play → pause/seek/volume, queue, real visualiser wired to real audio | **Met.** Verified incl. 9 live SIGWINCH resizes. Ratings pulled forward from M2. See [M1-STATUS.md](./M1-STATUS.md) |
| **M2 — Parity core (v1.0)** ✅ | Home, Library, Liked, artist/album/playlist pages, **thumbs up/down + library toggle**, radio/autoplay, shuffle/repeat, history, help overlay, themes, error toasts | **Met** for the acceptance test: a full listening session needs no browser. On-disk audio cache (FR-C2) deferred with reasons — see [M2-STATUS.md](./M2-STATUS.md) |
| **M3 — Polish (v1.1)** ✅ | Gapless, MPRIS, lyrics, playlist editing, command palette, mouse, session restore, `doctor` | **Met** except FR-A2 (keyring) and FR-B2 (moods & genres), both deferred with reasons in [M3-STATUS.md](./M3-STATUS.md) |
| **M4 — Delight (v1.2)** | Spectrogram mode, beat detection, crossfade, synced lyrics, Sixel art, scrobbling, CLI/IPC | Selected `C` items |
| **M5 — Hardening** | Fixture test suite for backend shapes, fuzz the parsers, packaging (AUR, brew, cargo-binstall), CI matrix | Reproducible releases |

**Suggested build order rationale:** M0 front-loads every risk that could kill the project (stream access, codec, visualiser feasibility) before any UI investment. `ytm-viz` first also means the signature feature gets tuned with a fast local feedback loop rather than being rushed at the end.

---

## 14. Glossary

- **InnerTube** — YouTube's private JSON API used by its own clients.
- **itag** — numeric id for a specific stream format/bitrate combination.
- **PO Token** — "proof of origin" token required since Aug 2024 for web-client stream access.
- **SAPISIDHASH** — cookie-derived request signature used for authenticated Google requests.
- **Feedback token** — opaque per-item token required for library add/remove operations.
- **Bark scale** — perceptual frequency scale approximating critical bands of human hearing.
- **Octant/sextant** — Unicode block glyphs giving 2×4 / 2×3 sub-cell resolution in a terminal.

---

## 15. Sources

- [rustypipe — docs.rs](https://docs.rs/rustypipe/latest/rustypipe/) · [Codeberg](https://codeberg.org/ThetaDev/rustypipe)
- [rustypipe RustyPipeQuery method reference](https://docs.rs/rustypipe/latest/rustypipe/client/struct.RustyPipeQuery.html)
- [ytmusicapi (Rust) — docs.rs](https://docs.rs/ytmusicapi)
- [ytmusicapi (Python) rating & library docs](https://ytmusicapi.readthedocs.io/en/stable/reference/library.html) · [FAQ / auth](https://ytmusicapi.readthedocs.io/en/stable/faq.html)
- [Symphonia codec support](https://github.com/pdeljanov/Symphonia)
- [rodio](https://github.com/RustAudio/rodio)
- [Ratatui v0.30 highlights](https://ratatui.rs/highlights/v030/) · [release notes](https://github.com/ratatui/ratatui/releases/tag/ratatui-v0.30.0)
- Prior art: [ytermusic](https://github.com/ccgauche/ytermusic) · [youtui](https://crates.io/crates/youtui) · [ytui-music](https://github.com/sudipghimire533/ytui-music) · [ytmusic-tui](https://lib.rs/crates/ytmusic-tui)
