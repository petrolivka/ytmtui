# Changelog

Notable changes to ytmtui. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows [semantic versioning](https://semver.org) from 1.0 onwards.

## [Unreleased]

Everything below is the pre-release development of 0.1.0, kept as a record of
what was built and what was learned.

### Added

- **Browsing** — Home feed, Explore, Charts, New releases, the full library
  (liked songs, playlists, albums, artists, history), and artist, album and
  playlist pages with back-navigation.
- **Moods & genres** — the mood and genre categories behind Explore (Chill,
  Focus, Workout, Sleep, Party, and the genre list) are browsable and open to
  their playlists. These are built from a renderer no other surface uses, and
  every category shares one browse id — they are told apart by an opaque
  `params` the browse request now carries.
- **Home filter chips** — the mood chips the feed comes with (`Relax`,
  `Energize`, `Workout`, `Commute`, `Focus`…) re-fetch Home slanted to one.
  `[` / `]` cycle them, the same keys as the search tabs.
- **Search** across songs, albums, artists and playlists, with debounced
  suggestions and recallable history.
- **Playback** — gapless handover, crossfade, seek, 0.5–2× speed with pitch
  preserved, loudness levelling, quality and output-device selection.
- **Queue** — play-next, enqueue, remove, shuffle, repeat, radio and autoplay
  when the queue runs dry.
- **Account** — thumbs up/down, add to library (kept distinct from a like),
  playlist create/rename/delete/add/remove, artist subscribe.
- **Visualiser** — bars, mirrored, oscilloscope, spectrogram, chroma, doom-fire
  and ink styles, with beat-driven accents and analyser constants tuned from
  measured percentiles. The oscilloscope, fire and ink are drawn as real pixels
  through sixel or the Kitty protocol, falling back to half blocks elsewhere.
- **Chroma** — the spectrum folded into twelve pitch classes and scrolled as a
  strip, so what shows is which notes are sounding rather than where the energy
  is. Rows run up the circle of fifths, which puts the seven notes of a key on
  seven neighbouring rows.
- **Album art** — half-block, sixel and Kitty graphics backends.
- **Lyrics** — plain from YouTube Music, time-synced from LRCLIB.
- **Integration** — MPRIS2, desktop notifications, ListenBrainz scrobbling, and
  a control socket driving a running instance from scripts.
- **Interface** — config file with themes and fully remappable keys, command
  palette, mouse support, session restore, generated help overlay.
- **Tooling** — `--doctor`, `--log-file`, `--anonymous`, `probe`, `tune`,
  `authcheck`, `dump-fixtures`.
- **Testing** — captured InnerTube fixtures with assertions on what parsers must
  extract, seeded mutation tests proving they never panic, and CI that runs both
  with no network.

### Notable fixes during development

- Pipe reads moved off the audio callback; a blocking read there caused real
  buffer underruns.
- Visualiser bars climbed while playback was paused, because the gain ceiling
  decayed toward a frozen buffer.
- `likeStatus` was read from watch-queue rows, which do not carry it, so every
  track reported as unrated.
- `pageType` was read by fixed path rather than searched for, so albums, artists
  and playlists were discarded everywhere.
- Track durations were parsed only from search responses, leaving library rows
  without a total and a progress bar that never filled.
- Four actions were added to the enum but not the palette catalogue, making them
  unreachable.
- Album art was stretched: every backend assumed a terminal cell is exactly
  twice as tall as it is wide, so the cover came out wider than it was tall in
  any font that is not 2:1. The cell is now measured, and the pane is fitted to
  a square in both directions; `art.cell_aspect` overrides the measurement
  where the terminal will not report one, tmux included.
- The previous visualiser style flickered through the fire. Cells holding a
  picture drawn as a raw escape are skipped by the renderer's diff, so they are
  never repainted - which keeps the picture alive across frames, but also meant
  whatever was drawn there before was never erased, and the terminal painted it
  back on top. The cells are now cleared once before being skipped, taken back
  through the diff when the picture goes away, and the Kitty transmit names a
  placement so each frame replaces the last instead of deleting it first.
- The oscilloscope was not one. It drew the band values mirrored about the
  midline - the spectrum in a different shape - rather than the waveform. The
  analyser now publishes the time-domain samples, and the trace is started at a
  rising zero crossing so it holds still.

### Security

- An automated UI test wrote a rating to a live account. `--anonymous` was added
  and all test tooling switched to it; see `docs/M4-STATUS.md`.
