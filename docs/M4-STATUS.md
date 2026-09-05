# M4 — Delight: Status

**Complete.** Every `C` item on the roadmap for this milestone is implemented,
including album art, which was descoped and then explicitly asked for.

| | |
|---|---|
| Date | 2026-09-05 |
| Follows | [M3-STATUS.md](./M3-STATUS.md) |

---

## 1. What was built

| Requirement | State |
|---|---|
| Spectrogram / waterfall | ✅ scrolling heat-map, two frequency bins per cell |
| FR-V9 beat detection | ✅ spectral flux with an adaptive threshold; border accents |
| FR-V10 album art | ✅ half-block, sixel and Kitty backends |
| FR-P6 crossfade | ✅ 0–12 s, two players on one mixer |
| FR-P7 playback speed | ✅ 0.5–2.0x, pitch preserved |
| FR-P11 loudness levelling | ✅ see the caveat in §3 |
| FR-Y2 synced lyrics | ✅ LRCLIB, active line highlighted |
| FR-I3 notifications | ✅ notify-send / terminal-notifier |
| FR-I4 scrobbling | ✅ ListenBrainz |
| FR-I5 CLI / IPC | ✅ control socket, `ytmtui status --json` |

## 2. Album art

Three backends, because terminals differ more than one would like:

- **Half blocks** work everywhere, including Alacritty, which supports no
  graphics protocol at all. A cell's foreground and background become two
  stacked pixels via `▀`; since a cell is about twice as tall as it is wide,
  those pixels come out square. A 20×10 pane is 20×20 real pixels — plenty for
  cover art, which is large flat colour rather than fine detail.
- **Sixel** for foot and friends, with a fixed palette (6×6×6 cube plus a grey
  ramp, since near-greys quantise badly on the cube) and run-length encoding.
- **Kitty graphics protocol**, chunked base64 PNG.

Detection is conservative on purpose: an unknown terminal gets half blocks
rather than a protocol that might print garbage into it.

## 3. Honest caveats

- **"Loudness levelling", not EBU R128 normalisation.** `loudnorm`'s dynamic
  mode buffers three seconds of lookahead, which would delay the start of every
  track, so this uses `dynaudnorm`. The stream's own `loudnessDb` is not exposed
  through the yt-dlp path, so there is nothing to normalise *to*.
- **Crossfade was verified as a clean transition, not measured acoustically.**
  Both players demonstrably run and the ramp is driven from the engine loop, but
  the two tracks used for testing both fade at their edges, so the overlap could
  not be confirmed from the spectrum alone.
- **The Kitty backend is implemented but untested**: there is no kitty,
  ghostty or wezterm on this machine to verify it renders. Half-block and sixel
  are exercised; sixel structurally, by tests over the encoder's output.

## 4. An incident worth recording

While testing the spectrogram, a screenshot showed a thumbs-down glyph on a
track nobody had rated. It was real: **an automated UI test had written a
dislike to the live account.** These tests type text into a running instance,
and one keystroke reached the app in normal mode rather than the search field,
where `d` is thumbs-down.

Fixed in three steps:

1. The rating was cleared, and `authcheck --rate` added so ratings can be set
   explicitly rather than only toggled.
2. The library was audited: playlist, album and artist counts were unchanged
   from the start of M2, so nothing else had been written.
3. **`--anonymous` was added and every driven test switched to it.** Account
   writes are now impossible during testing rather than merely unlikely.

The lesson generalises: a test harness that drives a real, authenticated
application is a production client. It needed the same guard rails as one.

## 5. Other bugs found and fixed

- **Four new actions were unreachable from the command palette.** They were
  added to the enum, but the edit adding them to `Action::ALL` silently did not
  apply, so the palette reported "no matching action". Now covered by tests:
  every action bound by the default keymap must be in the catalogue, the
  catalogue has no duplicates, and every action must be findable by typing its
  own label.
- **The crossfade opened its decoder on the engine thread**, stalling the very
  loop that drives the ramp — the fade would have jumped rather than glided.
  Moved to a background thread, as the gapless path already did.
- **The cover pane never appeared**, because the layout was keyed off the art
  having loaded while the fetch needed the pane's size. Now keyed off the track
  having art at all.
- **An unknown CLI word started a second player** instead of reporting a typo.

## 6. Verified live

```
spectrogram     scrolling heat-map renders and advances
beat detection  steady tone -> no onsets; pulsed signal -> onsets (tests)
album art       Windowlicker cover recognisable at 20x20 half-block pixels
crossfade       6 s overlap, clean handover into the next track
speed           1.20x shown, playback continued across the change
loudness        "norm" flag, decoder re-opened at position
synced lyrics   LRCLIB returned 77 / 15 / 29 lines for three tracks
notifications   notify-send path exercised
scrobbling      submission threshold covered by tests
IPC             status JSON parsed, pause/play/next/volume, clean errors
```
