# M3 — Polish: Status

**Complete.** Every `S` requirement in the analysis is met except two, both
listed with reasons in §4.

| | |
|---|---|
| Date | 2026-09-05 |
| Follows | [M2-STATUS.md](./M2-STATUS.md) |

---

## 1. What was built

| Requirement | State |
|---|---|
| FR-C1 config file, hot-reload | ✅ `config.toml`, reloaded on save |
| FR-U3 fully remappable keys | ✅ every binding resolves to an `Action` |
| FR-U7 themes | ✅ config-driven, three ready-made ones in `contrib/themes/` |
| FR-U5 command palette | ✅ `:` with subsequence matching |
| FR-U6 mouse | ✅ scroll, click-to-select, click-to-seek |
| FR-P5 gapless | ✅ next decoder queued ~12 s early |
| FR-P8 quality selector | ✅ `audio.quality` maps to an itag preference |
| FR-P9 output device | ✅ `audio.device`, devices listed by `--doctor` |
| FR-Q5 queue persistence | ✅ |
| FR-C5 session restore | ✅ restored paused |
| FR-C4 logging, doctor | ✅ `--log-file`, `--doctor` |
| FR-L4 playlist editing | ✅ create, rename, delete, add, remove |
| FR-R5 add to playlist | ✅ picker, with "new playlist" inline |
| FR-R6 subscribe | ✅ a real toggle, see §3 |
| FR-R8 copy link | ✅ wl-copy / xclip / xsel / pbcopy |
| FR-S2 search suggestions | ✅ debounced 250 ms |
| FR-S3 search history | ✅ recalled with Up/Down |
| FR-Y1, FR-Y3 lyrics | ✅ panel, "no lyrics" handled as ordinary |
| FR-I1 MPRIS2 | ✅ playerctl, media keys, desktop widgets |

## 2. Design notes worth keeping

**Keys are data.** Every binding resolves to an `Action`, which is what makes
the config, the command palette and the generated help overlay all the same
mechanism rather than three parallel lists that drift apart. The help overlay
is built from the live keymap, so it shows a user's rebindings.

**A broken config is not fatal.** A config that fails to parse is reported and
ignored; individual bad entries are skipped with their own warning while the
rest still applies. Losing an entire config to one typo would be a poor trade.
Verified with two deliberately broken bindings: two warnings, 52 bindings still
active.

**Gapless without a queue-editing API.** rodio can clear its whole queue but
cannot remove one entry behind the playing source. So when what plays next
changes while a decoder is already queued, the current track is restarted at its
position instead. That is rare by construction, since arming only happens in the
last few seconds of a track.

**Mouse mapping uses real scroll offsets.** The row under the pointer is
`ListState::offset() + row`, not an estimate from the selection; estimating
picks the wrong row as soon as a list is scrolled. Verified by scrolling, then
clicking the third visible row and playing exactly that track.

**MPRIS is best-effort.** No session bus means no media-key integration, not a
failure to start.

## 3. Bugs found and fixed during M3

- **`--list-actions` printed names the config would reject.** Names were derived
  from `Debug` ("togglepause") while the parser expects serde's snake_case
  ("toggle_pause"). Now covered by a test that round-trips every action name
  through the real parser.
- **A plain `G` binding could never match.** Terminals report `G` with the shift
  flag set, so a chord carrying shift never equalled the configured one. Shift
  is now dropped for character keys and kept for the rest, with tests both ways.
- **`"+"` could not be bound.** Splitting on `+` left an empty key name.
- **Subscribe could only ever subscribe.** A "toggle" with no read of the
  current state is not a toggle; it now reads first.
- **Suggestions were painted over by the body panes.** They are drawn last, like
  the other overlays.
- **Piping `--list-actions` into `head` panicked** on a broken pipe.

## 4. Deferred, deliberately

| Requirement | Why |
|---|---|
| FR-A2 OS keyring | Cookies are stored in a file tightened to 0600. A keyring pulls in a platform-specific dependency and a headless-session failure mode; worth doing, but on its own and not as an afterthought at the end of a milestone. |
| FR-B2 moods & genres | `FEmusic_moods_and_genres` returns nothing to the probe and needs its own renderer. Explore, Charts and New releases all work. |
| FR-C2 on-disk audio cache | Carried over from M2 for the same reason: it is a large piece that sits close to the "no downloads" line in the risk analysis, and wants its own design pass. |
| FR-Y2 synced lyrics | Priority C. Needs a second provider (LRCLIB); plain lyrics are in. |

## 5. Verified live

```
config          two broken bindings -> two warnings, 52 bindings still active
hot-reload      editing config.toml -> "config reloaded" without a restart
doctor          ffmpeg, yt-dlp, 34 audio devices, truecolor, account name
gapless         Father -> Avril 14th with no buffering state, spectrum unbroken
MPRIS           busctl: PlayPause flipped Playing/Paused, Next moved track,
                metadata carried title, artist, album and URL
mouse           scrolled, clicked the 3rd visible row, Enter played that track
palette         ":" then "shuf" -> Toggle shuffle, with its binding shown
playlists       picker listed the account's playlists, "new playlist" inline
lyrics          real lyrics for a vocal track, "no lyrics" for an instrumental
session         20-track queue restored paused at 0:15
suggestions     "boards of ca" -> five suggestions, Tab accepted the first
```
