# M2 — Parity Core: Status

**Complete for the milestone's acceptance test: a full listening session no
longer needs a browser.** Home, Explore, the library, artist/album/playlist
pages, search across four tabs, radio/autoplay, ratings and the library toggle
all work against a real account.

| | |
|---|---|
| Date | 2026-09-05 |
| Follows | [M1-STATUS.md](./M1-STATUS.md) |

---

## 1. What was built

| Surface | State |
|---|---|
| **Home feed** | ✅ real shelves — Featured playlists, Listen again, mixes |
| **Explore / New releases / Charts** | ✅ |
| **Search** | ✅ four tabs: Songs, Albums, Artists, Playlists (`[` `]` to switch) |
| **Library** | ✅ Liked songs, Playlists, Albums, Artists, History |
| **Artist page** | ✅ albums, singles, related |
| **Album page** | ✅ tracklist, artist filled in from the header |
| **Playlist page** | ✅ tracklist |
| **Navigation** | ✅ sidebar + stack with `Esc` back, cursor position preserved |
| **Context navigation** | ✅ `g` / `G` go to artist / album of the selection |
| **Continuations** | ✅ next page pulled in as the cursor nears the end |
| **Ratings** | ✅ thumbs up/down with true state fetched per track |
| **Library toggle** | ✅ `a`, kept distinct from thumbs-up |
| **Radio / autoplay** | ✅ (from M1) |

Everything renders through a single `Row` type, so navigation, selection and
drawing are written once rather than per surface.

## 2. The parser findings, all from evidence

The `probe` diagnostic exists because guessing at InnerTube's shapes is how this
goes wrong. Each of these was a silent empty page until probe showed it:

- **`pageType` is buried** at `navigationEndpoint.browseEndpoint.
  browseEndpointContextSupportedConfigs.browseEndpointContextMusicConfig.pageType`.
  Reaching for it by fixed path missed it, so albums, artists and playlists were
  discarded everywhere — the library and three of four search tabs returned
  nothing at all.
- **`FEmusic_library_corpus_track_artists` is dead.**
  `FEmusic_library_corpus_artists` returns the library's artists.
- **Two-row cards carry `subtitle`**, not `flexColumns`.
- **Album subtitles differ by surface**: "Album • Aphex Twin • 1992" in search
  but a bare "2001" on an artist page, so taking the first field put a year in
  the artist column. Covered by a test.
- **Album tracklists omit the per-row artist**, since it is constant; it is now
  filled from the page header.
- **`likeStatus` is not in the queue rows.** It lives at
  `playerOverlays.playerOverlayRenderer.actions[0].likeButtonRenderer.likeStatus`.
  See §3 — this one mattered.

## 3. A wrong verification, and what it hid

`authcheck` verified the write path by checking whether a track appeared in the
Liked Songs list. Two problems, discovered together:

1. **`track_state` reported `Indifferent` for everything**, because it read
   `likeStatus` from the watch-queue rows, which do not carry it. Any rating
   shown in the UI would have been wrong.
2. **Liked Songs is a derived auto-playlist and lags by minutes.** A track
   un-liked minutes earlier was still listed. Verifying against it reports
   failures that did not happen — and would equally hide real ones.

Both fixed. `track_state` now reads the player overlay, and cross-checking
proved it: two tracks known to be liked report `Like`, and the test track from
M0 reports `Indifferent`, confirming that M0's un-like *did* take effect and the
account was left as found. `authcheck` now verifies against the track's own
rating and restores the previous value rather than assuming it was unrated.

**Lesson worth keeping:** a test that passes for the wrong reason is worse than
no test. The first regression test written for the paused-visualiser bug in M1
had the same defect — it passed with the fix disabled. Both were caught by
deliberately breaking the fix and watching the test go red, which is now the
habit for anything claiming to verify behaviour.

## 4. Account safety (R11)

Two mitigations landed with this milestone, both required by FR-N1/N2:

- **Response caching** with per-surface TTLs (search 5 min, home/explore 15 min,
  library 2 min, entity pages 24 h). Repeat navigation does not re-hit the API.
- **Request pacing**: a minimum 120 ms spacing between requests, so a burst of
  navigation cannot look like scraping.

Also: the cookie file's permissions are tightened to 0600 on load if they are
broader (FR-A2), and a 401/403 now says the session expired and what to do about
it rather than surfacing a bare status code (FR-A4).

## 5. Deferred, deliberately

| Requirement | Why |
|---|---|
| FR-C2 on-disk audio cache (M) | Metadata caching landed; caching *audio* to disk is a much larger piece and sits close to the "no downloads" line in §5 of the risk analysis. Wants its own design pass. |
| FR-L4 playlist create/edit (S) | Write surface beyond ratings; M3. |
| FR-Y lyrics (S) | M3. |
| FR-U7 themes from config (S) | The `Theme` struct is the single source of colour; loading it from TOML is M3 work alongside the config file. |
| FR-S2 search suggestions (S) | M3. |
| FR-B2 moods & genres | `FEmusic_moods_and_genres` returned nothing to the probe; needs its own renderer. Explore, Charts and New releases all work. |

## 6. Verified live

```
home            7 tracks, 2 albums, 31 playlists, 3 shelves      (+continuation)
search          Songs 20 / Albums 20 / Artists 2 / Playlists 20  (+continuation)
library         Liked 25, Playlists 25, Albums 21, Artists 25, History 200
artist page     Charlotte de Witte -> 24 tracks, 12 albums, 20 playlists
album page      True Electric -> 38 tracks
playlist page   Liked Music -> 100 tracks                        (+continuation)
navigation      sidebar -> Artists -> artist page -> Esc, cursor preserved
playback        Enter on Liked songs -> queue of 25, playing
context nav     g -> artist page of the playing track
rating state    two known-liked tracks report Like; the M0 test track Indifferent
```
