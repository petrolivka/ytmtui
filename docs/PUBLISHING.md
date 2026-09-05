# Publishing checklist

What still needs a human decision before this repository goes public. Nothing
here is a code change; it is all things only the owner can settle.

The repository is `https://github.com/petrolivka/ytmtui`, already set as
`origin`, and every URL in the tree points at it.

## Done

- Repository URL: `github.com/petrolivka/ytmtui`, set as `origin`, referenced
  everywhere in the tree.
- Private vulnerability reporting is enabled, so the advisory link in
  `SECURITY.md` and the issue template resolves.
- **Licence: GPL-3.0-or-later, decided.** Nothing forced it — all 336
  transitive dependencies are permissive (MIT, Apache-2.0, Zlib, ISC,
  Unicode-3.0). It was chosen because ytmtui is an application rather than a
  library: the only realistic derivative is a fork, so copyleft costs almost
  nothing and keeps improvements available.

  Worth knowing for later: relicensing to something permissive would now need
  every contributor's agreement, so this is effectively settled once the first
  outside pull request lands.

## Must do

| | Where | Why |
|---|---|---|
| **Confirm the maintainer email** | `contrib/packaging/PKGBUILD` | It is the git author address, so it is already in the history — but a package file publishes it more prominently, and that invites spam. Substitute an alias if you would rather. |

## Decide deliberately

### The committed fixtures

`crates/ytm-api/tests/fixtures/` holds 2 MB of real InnerTube responses: track
titles, artist names, thumbnail URLs. They are anonymous captures with tracking
fields stripped, and they are what makes the parser suite meaningful and
offline.

It is still third-party API response data in a public repository. That is a
judgement call, not a settled question:

- **Keep them** — the tests work on a fresh clone and in CI, and a shape change
  fails loudly. This is the current setup.
- **Stop tracking them** — add the directory to `.gitignore` and have
  contributors run `cargo run --bin dump-fixtures` first. The suite then needs
  network access once per clone, and CI needs a step to generate them, which
  reintroduces exactly the "is CI red because YouTube changed?" problem the
  fixtures exist to remove.

### The name

`ytmtui` is unclaimed on crates.io as far as this project knows, but that has
not been verified. Check before a first publish.

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
