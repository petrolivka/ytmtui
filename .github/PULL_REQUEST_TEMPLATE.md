## What this changes

<!-- And why. If the reason is non-obvious - which, on an undocumented API, it
usually is - that reason is the most valuable part of this description. -->

## How it was verified

<!-- Not "it compiles". For UI changes, paste the captured screen. For parser
changes, say which fixture test covers it. -->

## Checklist

- [ ] `cargo fmt --all` and `cargo clippy --workspace --all-targets` are clean
- [ ] `cargo test --workspace` passes
- [ ] Any script driving the app uses `--anonymous` (see CONTRIBUTING.md)
- [ ] New parsers are exercised in `crates/ytm-api/tests/robustness.rs`
- [ ] No downloading, exporting or ripping functionality
