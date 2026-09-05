# M5 — Hardening: Status

**Complete.** The project now has a test suite that fails when YouTube changes
shape rather than when the network is down, a clean clippy and fmt baseline, CI,
and packaging for three channels.

| | |
|---|---|
| Date | 2026-09-05 |
| Follows | [M4-STATUS.md](./M4-STATUS.md) |

---

## 1. Fixture suite — the point of the milestone

R1, "InnerTube response shapes change without notice", has been the project's
most persistent risk since the analysis. It is now testable.

`cargo run --bin dump-fixtures` captures ten real responses — four search tabs,
three discovery surfaces, an artist page, an album page and a watch-next queue —
and `crates/ytm-api/tests/fixtures.rs` asserts what the parsers must get out of
them. Not just "did not crash": at least 15 songs from a song search, 80% of
them carrying a duration and an artist, albums that are albums rather than
tracks, an artist page built from titled shelves, a queue of playable ids.

Two properties make this worth having:

- **Fixtures are anonymous captures.** Authenticated responses are full of
  personal data and these files are committed. Anonymous also means they cover
  exactly the shapes an unauthenticated user sees, which is most of the parser
  surface.
- **Tracking fields are stripped** — `trackingParams`, `clickTrackingParams`,
  `visitorData`, `responseContext` and friends. They are most of the bytes (3.0
  MB down to 2.0 MB) and carry per-session identifiers that have no business in
  a repository. Nothing stripped is read by any parser.

There is a test asserting `playerOverlays` exists in the watch-next response,
because that is where `likeStatus` lives — the M2 bug where ratings read as
Indifferent for everything would now fail here rather than in the UI.

## 2. Robustness

`tests/robustness.rs` mutates the fixtures with a seeded generator: dropping
keys, nulling values, swapping types, emptying arrays and blanking strings.
Each has a real-world analogue in a shape change. Every parser is run over the
result, and anything that survives must still be internally consistent — a
`Row::Track` only exists if it had a usable id and a non-empty title.

Deterministic, not random: a failure is reproducible from the seed in the
message. Degenerate documents (nulls, bare scalars, renderers with fields of the
wrong type, deep empty nesting) and truncated JSON are covered separately.

Real fuzz targets live in `fuzz/` for the response parser, the LRC parser and
the key-binding parser. They need nightly and `cargo-fuzz`, so they are not in
CI; the deterministic suite covers the same ground on every push, and fuzzing is
for what it misses. The LRC target asserts timestamps come out ordered and the
binding target asserts chords round-trip through their rendered form.

## 3. CI

Two workflows:

- **CI** — fmt, clippy (`-D warnings`), build and test on Linux and macOS,
  plus a Linux run pinned to the 1.85 MSRV so a bump has to be deliberate.
- **offline-guarantee** — runs the parser suites inside `unshare -rn`, a
  network-free namespace. **A red CI must mean our code broke, not that an
  undocumented API changed overnight.** Verified locally: 12 tests pass with no
  network at all.
- **Release** — tagged builds for x86-64 and aarch64 Linux and Apple Silicon,
  producing archives whose names match the `cargo-binstall` metadata, with
  checksums.

## 4. Packaging

| Channel | File |
|---|---|
| Arch / AUR | `contrib/packaging/PKGBUILD` |
| Homebrew | `contrib/packaging/ytmtui.rb` |
| cargo-binstall | `[package.metadata.binstall]` in `Cargo.toml` |

Both packages declare `ffmpeg` and `yt-dlp` as **runtime dependencies rather
than suggestions**: the app cannot decode or resolve a stream without them, so
treating them as optional would ship a broken install.

The release profile uses thin LTO, one codegen unit and stripped symbols, and
deliberately keeps `panic = "unwind"` — the panic hook restores the terminal,
and `abort` would skip it, leaving a wrecked terminal behind.

## 5. Licence

`LICENSE` now carries GPL-3.0-or-later, matching what every crate manifest has
declared since M1.

Worth restating: **this is still a free choice.** The GPL constraint in the
original analysis came from `rustypipe`, which the project does not use — reads
go through the in-house InnerTube client. Nothing in the dependency tree is
copyleft. Changing it means editing `license` in `Cargo.toml`, replacing
`LICENSE`, and updating both packaging files.

## 6. Quality baseline

```
clippy      0 findings across the workspace, all targets
rustfmt     clean
tests       53 passing, of which 12 need no network
build       0 warnings
```

Clippy found two enums whose variants differed hugely in size — `Modal` and
`Prompt`, both moved on every keystroke while an overlay is open. Their `Track`
payloads are now boxed.

## 7. Deferred

| | Why |
|---|---|
| Windows CI | Nothing has been tested on Windows and the audio path is untried there; a green tick would claim more than is true. |
| Publishing to crates.io / AUR / Homebrew | Needs accounts, a repository URL that exists, and a tagged release. The recipes are ready and reference `github.com/OWNER/ytmtui`. |
| Fuzzing in CI | Wants nightly and a corpus that grows across runs; better as a scheduled job once the project has a home. |
