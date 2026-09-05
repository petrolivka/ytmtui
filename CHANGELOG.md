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
- **Search** across songs, albums, artists and playlists, with debounced
  suggestions and recallable history.
- **Playback** — gapless handover, crossfade, seek, 0.5–2× speed with pitch
  preserved, loudness levelling, quality and output-device selection.
- **Queue** — play-next, enqueue, remove, shuffle, repeat, radio and autoplay
  when the queue runs dry.
- **Account** — thumbs up/down, add to library (kept distinct from a like),
  playlist create/rename/delete/add/remove, artist subscribe.
- **Visualiser** — bars, mirrored, oscilloscope and spectrogram styles, with
  beat-driven accents and analyser constants tuned from measured percentiles.
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

### Security

- An automated UI test wrote a rating to a live account. `--anonymous` was added
  and all test tooling switched to it; see `docs/M4-STATUS.md`.
