# M1 — Skeleton Player: Status

**Complete.** The spike is retired and its code now lives in the real crate
workspace. The M1 acceptance criteria — *"can find and play a song and watch it
dance; no crashes on resize"* — are met and verified.

| | |
|---|---|
| Date | 2026-09-04 |
| Supersedes | `spike/` (deleted; its two useful tools were promoted to real binaries) |

---

## 1. Acceptance criteria

| Criterion | Result |
|---|---|
| Workspace layout per §9.1 | ✅ 5 crates + binary, `ytm-viz` still I/O-free |
| Search | ✅ live InnerTube search, 20 real results with titles/artists/durations |
| Play / pause / seek / volume | ✅ verified: seek ±5s/±30s, volume to 105%, progress bar tracks |
| Queue | ✅ populated, reorder-free ops (jump, remove, play-next, enqueue) |
| Visualiser on real audio | ✅ 128 bands, live, re-bins on resize |
| **No crashes on resize** | ✅ **9 live SIGWINCH resizes incl. 8×30 and 60×200 — no panic, clean exit** |

Beyond the milestone: **thumbs up/down were pulled forward from M2**, since the
API layer already implemented them and they are the feature that motivated the
project. They degrade to a clear message when running anonymously.

## 2. What was built

```
crates/ytm-core    domain types: Track, VideoId, Rating, RepeatMode, PlayerStatus
crates/ytm-viz     Analyser, SpectrumFrame  (no I/O - tunable against local files)
crates/ytm-api     Innertube (search/auth/rate), MusicBackend trait, defensive JSON walk
crates/ytm-player  StreamResolver + ResolverCache, FfmpegPcm + tap, engine thread
crates/ytm-tui     App/event loop, ui rendering, Spectrum widget, Theme
src/main.rs        wiring + panic hook that restores the terminal
src/bin/tune.rs    analyser tuning instrument (promoted from the spike)
src/bin/authcheck.rs credentialed self-test (promoted from the spike)
```

Concurrency is message-passing exactly as specified: the engine owns all
playback state and publishes an immutable `PlayerStatus`; the analyser publishes
`SpectrumFrame` via `ArcSwap`; the UI renders snapshots and never blocks on I/O.
Search runs on a worker thread and reports back by channel.

## 3. Defects found and fixed during M1

**(a) Blocking pipe read on the audio thread — the serious one.**
The spike's `Source::next()` read directly from ffmpeg's stdout. Since rodio
calls `next()` on the audio callback, any network hiccup stalled the sound card;
the app reported `audio stream error: Buffer underrun/overrun occurred`. This
violated NFR-4 outright.

Fixed by moving the pipe read to a dedicated decode thread feeding a ~4 s
lock-free ring. `next()` now only pops from the ring: it never blocks, never
allocates, emits silence on a brief underrun, and returns `None` on genuine EOF
or after a 15 s stall so a dead stream cannot hang the queue. `open()`
prebuffers 0.5 s before playback starts.

**(b) rodio printing over the TUI.**
rodio `eprintln!`s device errors when its `tracing` feature is off, painting
raw text across the interface. Fixed by enabling that feature — and while there,
dropping rodio's decoders entirely (`default-features = false`), since ffmpeg
does all decoding. Smaller dependency tree, no Symphonia at all.

**(c) Progress bar never rendered.** The now-playing block was 3 rows, leaving
1 inner row — enough for the title, nothing for the bar. Now 4.

**(d) `authcheck` was destructive and its verification was unsound.**
It always cleared the rating at the end, which would have *deleted an existing
like* if the test track was already liked; and it confirmed success by comparing
the length of the liked list, which cannot change once the first page is full.
Now it records whether the track was already liked, verifies by presence of that
specific video id, and restores exactly the prior state.

**(e) Column alignment broken by CJK titles.** Truncation counted `char`s
instead of display columns, so any result with wide characters (real search
results routinely have them) skewed the whole list. Now width-aware via
`unicode-width`.

**(f) Panicked instead of explaining when there was no TTY.**
Launching from an agent shell, an editor, a pipe or cron hit
`ratatui::init()`'s internal `expect`, producing a raw panic and stray escape
codes. `main` now checks `IsTerminal` before opening the audio device, and the
UI uses `try_init()`, so a headless run exits 1 with a plain explanation. This
was a straight NFR-5 violation ("no panics reach the user").

**(i) Track durations were missing everywhere except search.** Reported from
real use: every row in Liked Songs showed `--:--`. Search returns the duration
in the last run of `flexColumns[1]`, but library and playlist responses put it
in `fixedColumns`, and the parser only read the former. Without a duration the
progress bar has no total and never fills, so this was functional rather than
cosmetic. Duration is now read from `flexColumns`, then `fixedColumns`, then by
scanning the row; `parse_duration` was tightened to match, since the new
fallback scan would otherwise turn any colon-bearing text into a bogus
duration.

**(g) Bars climbed while playback was paused.** Reported from real use. With
playback paused the tap goes quiet, so `analyse` was called repeatedly against a
frozen history buffer while the automatic-gain ceiling decayed toward it — every
band was therefore measured against a shrinking reference and *rose*, on a track
that was silent.

Fixed inside the analyser rather than at the call site, so no caller can
reintroduce it: `analyse` now detects that nothing was fed since the previous
pass and shifts silence into the history, and the AGC ceiling has a hard floor
(`AGC_FLOOR`) so near-silence can never be amplified. Covered by
`bands_never_climb_while_no_audio_arrives`, which fails without the fix.
Verified in the running app: after pause the spectrum decays 54 → 44 → 19 → 2 →
0 filled cells and stays at 0.

**(h) The mirrored visualiser read as a solid slab.** Reported from real use.
Three causes, all fixed: the two halves were an exact mirror with no separator,
so the innermost rows fused into one block; bands were drawn one per column with
no gutter, so neighbours merged into a continuous mass; and peak caps were drawn
inside filled columns, adding speckle. Now the real bars take two thirds of the
height, a blank baseline row separates the halves, the reflection is drawn at
55% amplitude and 45% brightness, bars get a one-column gutter wherever width
allows, and caps are drawn only when clear of the bar.

## 4. Notable design decisions

- **Frame-rate cap is enforced (60 fps).** M0 measured ~119 fps uncapped for
  roughly double the visualiser CPU and no visible benefit. The UI spends the
  remainder of each frame blocked in `poll_input`, so an idle app costs nothing
  while a keypress is still handled immediately.
- **One tap for the app's lifetime.** The visualiser is never rewired on track
  change; the decoder writes into a shared producer with `try_lock`, which is
  effectively uncontended since only one decoder runs at a time.
- **Prev restarts the track if >3 s in**, matching the official player.
- **Thumbs-down skips**, also matching the official player (FR-R2).
- **Minimum size is 44×10**; below that the UI says so rather than rendering
  garbage. The queue pane drops out below 92 columns.

## 5. R7 (GPL propagation) is currently void

The licence risk in the analysis came from `rustypipe`, which is GPL-3.0. M1
does not use it: search, auth and ratings are served by the in-house InnerTube
client instead, so **there is presently no GPL dependency and the licence is a
free choice**. The workspace is set to `GPL-3.0-or-later` as a placeholder from
the original plan — change it if you prefer something permissive. Adopting
rustypipe later for richer reads (Home, artist pages, radio) would reintroduce
the constraint, so it is worth deciding deliberately before M2.

## 6. Still open

- ~~R11 / the credentialed test.~~ **Done 2026-09-05** — authenticated read and
  the like/unlike write path both confirmed on a real account. Every path in the
  project is now verified end to end.
- Home feed, Explore, artist/album/playlist pages (M2).
- Radio/autoplay when the queue drains — currently playback simply stops.
- Add-to-library (distinct from thumbs up), playlist editing, lyrics.
- Config file, themes, remappable keys, session restore (M3).
- Gapless playback: track changes currently tear down and respawn ffmpeg.

## 7. Verified behaviour

```
search "daft punk one more time"  -> 20 results, correct metadata
play                              -> itag 251 opus 129k, spectrum live
seek +10s, volume 105%            -> progress bar 0:23 / 4:54, vol 105%
resize x9 (8x30 .. 60x200)        -> no panic, playback continued, exit 0
30x8 terminal                     -> "terminal too small, need 44x10"
help overlay                      -> renders centred, any key closes
tune dQw4w9WgXcQ                  -> p50=0.456, saturated 0.8% (matches M0 tuning)
```
