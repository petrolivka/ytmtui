# Contributing to ytmtui

Thanks for looking. This document is short on ceremony and long on the two or
three things that are genuinely easy to get wrong here.

## Getting set up

```bash
cargo build
cargo test --workspace
cargo run --bin ytmtui -- --doctor
```

`ffmpeg` and `yt-dlp` must be on `PATH`. `--doctor` tells you what is missing.

## Before opening a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets   # must be clean; CI runs with -D warnings
cargo test --workspace
```

## Two rules that matter more than the rest

### 1. Never run automated tests against a signed-in instance

This is not hypothetical. During development, an automated UI test typed text
into a running, authenticated instance; one keystroke landed in normal mode
rather than the search field, where `d` is thumbs-down, and it wrote a dislike
to a real YouTube account.

**Any script that drives the application must pass `--anonymous`.** That makes
account writes impossible rather than merely unlikely.

```bash
./target/release/ytmtui --anonymous
```

The parser test suites need no account and no network at all — that is the point
of the captured fixtures.

### 2. Parsers must degrade, never panic

InnerTube is undocumented and changes without notice. "The shape is wrong" is an
expected condition, not an exceptional one. A panic takes the whole player down
mid-listen; an empty pane does not.

Every accessor returns an `Option`, unknown renderers are skipped, and
`crates/ytm-api/tests/robustness.rs` mutates real responses to prove it. If you
add a parser, add it to `exercise()` there.

## Working on the InnerTube layer

Response shapes are the project's most persistent risk, so they are pinned by
fixtures rather than trusted:

```bash
cargo run --release --bin probe           # read-only tour of every API surface
cargo run --release --bin dump-fixtures   # refresh crates/ytm-api/tests/fixtures
cargo test -p ytm-api                     # assert what the parsers must extract
```

`dump-fixtures` captures **anonymous** responses only and strips tracking
fields. Never commit a fixture captured while signed in — those responses are
full of personal data.

If a fixture test fails after a refresh, YouTube changed something. Fix the
parser; do not relax the assertion until it passes.

## Testing the interface

There is no UI test framework here. The interface is driven through a pseudo-
terminal and the resulting screen is reconstructed and asserted on — see the
milestone documents for how each feature was verified. If you add UI, drive it
the same way and paste the captured screen into the pull request.

## Audio and the realtime path

`FfmpegPcm::next()` runs on the audio callback. It must never block, allocate,
lock, or log. Reading from the ffmpeg pipe happens on a separate thread feeding
a lock-free ring for exactly this reason; a blocking read there caused real
buffer underruns before it was moved.

## Commit messages

Say what changed and *why*, especially when the reason is non-obvious — which,
in a project built on an undocumented API, it usually is. If you discovered
something about InnerTube, write it down: that knowledge is most of the value.

## Fuzzing

```bash
cargo install cargo-fuzz
rustup toolchain install nightly
cargo +nightly fuzz run parse_response
```

See [`fuzz/README.md`](fuzz/README.md). CI runs the deterministic equivalent on
every push.

## Scope

Please do not propose downloading, exporting, or ripping features. Keeping the
traffic pattern squarely "playback" is what keeps this defensible, and it is
also the honest description of what the project does.
