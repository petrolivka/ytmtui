# Fuzzing

Requires nightly and `cargo-fuzz`, so it is not part of CI:

```bash
cargo install cargo-fuzz
rustup toolchain install nightly

# Seed from the captured fixtures rather than from nothing.
mkdir -p fuzz/corpus/parse_response
cp ../crates/ytm-api/tests/fixtures/*.json fuzz/corpus/parse_response/

cargo +nightly fuzz run parse_response
cargo +nightly fuzz run parse_lrc
cargo +nightly fuzz run parse_binding
```

CI covers the same ground deterministically with
`crates/ytm-api/tests/robustness.rs`, which mutates the fixtures from a seeded
generator — reproducible, no nightly, and fast enough to run on every push.
Fuzzing is for finding what that misses.
