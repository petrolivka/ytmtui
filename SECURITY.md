# Security policy

## Reporting a vulnerability

Please report security issues privately — open a **GitHub security advisory**
on this repository rather than a public issue.

Include what you did, what happened, and what you expected. A proof of concept
helps; a working exploit is not required and is not wanted for anything that
could affect other people's accounts.

## What this project handles that is worth caring about

ytmtui holds a **live Google session cookie**. That is the highest-value secret
in the project, and everything below follows from it.

| | |
|---|---|
| **Where credentials live** | `~/.config/ytmtui/cookies.txt`, or the `YTM_COOKIE` environment variable |
| **Permissions** | The file is tightened to `0600` on load if it is broader |
| **Logging** | Credentials are never written to logs, and `--log-file` output is not exempt from that |
| **Transport** | HTTPS with certificate verification, always; there is no option to disable it |
| **Scope** | Credentials are sent only to `music.youtube.com`, never to LRCLIB or ListenBrainz |

The `~/.local/state/ytmtui/session.json` file is not a credential, but it does
record what was listened to, so it is written `0600` as well.

## Please report

- Any path by which a credential could leak into a log, a crash report, a
  committed file, or a request to a third party.
- Anything that makes the client's traffic look less like a player and more like
  a scraper, since that is what puts a user's account at risk.
- A parser input that causes a panic, a hang, or unbounded memory growth.
  Fixture-based and mutation tests cover this, but they will not have found
  everything.
- Command injection through a track title, a URL, or a config value reaching a
  subprocess (`ffmpeg`, `yt-dlp`, the clipboard tools, the notifier).

## Known and accepted risks

These are documented rather than fixed, and are not vulnerabilities in this
project:

- **Using a real account with an unofficial client risks account restriction.**
  This is inherent, documented in the README, and the reason `--anonymous`
  exists. It is not something a code change can remove.
- **Credentials are stored in a file, not an OS keyring.** A keyring backend is
  a known gap, tracked as FR-A2.
- **`ffmpeg` and `yt-dlp` are executed as subprocesses.** They are invoked with
  argument arrays, never through a shell, so a hostile track title cannot inject
  a command — but they are still third-party binaries running on your machine.

## Supported versions

The project is pre-1.0. Fixes land on the default branch; there are no
backported security releases yet.
