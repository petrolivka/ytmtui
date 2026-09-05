# Publishing checklist

What still needs a human decision before this repository goes public. Nothing
here is a code change; it is all things only the owner can settle.

The repository is `https://github.com/petrolivka/ytmtui`, already set as
`origin`, and every URL in the tree points at it.

## Decided

- **Repository**: `github.com/petrolivka/ytmtui`, set as `origin`, referenced
  everywhere in the tree.
- **Private vulnerability reporting**: enabled, so the advisory link in
  `SECURITY.md` and the issue template resolves.
- **Licence: GPL-3.0-or-later.** Nothing forced it — all 336 transitive
  dependencies are permissive (MIT, Apache-2.0, Zlib, ISC, Unicode-3.0). Chosen
  because ytmtui is an application rather than a library: the only realistic
  derivative is a fork, so copyleft costs almost nothing and keeps improvements
  available.

  Effectively irreversible once an outside contribution lands — relicensing
  away from GPL would need every contributor's consent.
- **Maintainer contact**: `petr.olivka@gmail.com`, as in the PKGBUILD and the
  git history.
- **Fixtures stay committed.** `crates/ytm-api/tests/fixtures/` holds 2 MB of
  anonymous InnerTube responses with tracking fields stripped. Keeping them
  means the parser suite works on a fresh clone and in CI with no network, so a
  red build means the code broke rather than that YouTube changed overnight —
  which is the whole reason the suite exists. Refresh them with
  `cargo run --release --bin dump-fixtures`; never commit one captured while
  signed in.
- **Name**: `ytmtui` is free on crates.io.

## Before a first release

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets     # must be clean
cargo test --workspace
cargo publish --dry-run
```

Then tag `v0.1.0`; the release workflow builds Linux x86-64, Linux aarch64 and
Apple Silicon binaries and attaches them with checksums, in the archive layout
that `cargo binstall` expects.

Packaging recipes need their checksums filled in once a tag exists:
`sha256sums` in the PKGBUILD and `sha256` in the Homebrew formula.

## Not claimed, deliberately

- **No Windows support.** Nothing has been tested there and the audio path is
  untried; CI does not build it, because a green tick would claim more than is
  true.
- **The Kitty art backend is unverified.** It is implemented but no terminal
  speaking that protocol was available to confirm it renders.
