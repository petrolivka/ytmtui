# ytmtui — Technology Choice: Risk Analysis

**Question:** does moving off Rust reduce the risk of shipping what we want?

**Short answer:** No — but the question surfaced something more valuable. The dominant risks are **not language-dependent**, and the biggest available risk reduction comes from an **architecture change we can make while staying in Rust**. One competing stack (Python) is genuinely better on API coverage, and one (TypeScript) is genuinely better on stream access; neither is better enough to outweigh what they cost us on the visualiser, which is the whole point of the product.

| | |
|---|---|
| Status | Research complete — recommendation below |
| Date | 2026-09-04 |
| Companion to | [ANALYSIS-AND-REQUIREMENTS.md](./ANALYSIS-AND-REQUIREMENTS.md) |
| Verdict | **Keep Rust. Change the architecture (§5).** Revised risk profile in §7 |

---

## 1. First: sort the risks by whether language can even affect them

This is the step that answers the question. From §11 of the requirements doc:

### Group A — Google-side. **Identical in every language.**

| Risk | Why no stack helps |
|---|---|
| R1 InnerTube response shapes change | Every client parses the same renderer trees. A Python, Rust, Go or TS client breaks the same day. Only *how fast someone else fixes it for you* differs (that's Group C). |
| R2 PO token / bot detection tightening | Server-side. Notably, current guidance on the leading PO-token provider is blunt: *"Passing PO tokens no longer bypasses the bot check for the majority of cases"* and *"does not guarantee bypassing 403 errors."* No language fixes this. |
| **R11 (new) Account ban risk** | yt-dlp's own documentation warns that using your account with third-party tooling *"runs the risk of it being banned (temporarily or permanently)"*, and OAuth login was killed — cookies are the only path. Since we specifically want likes to sync to the **real** account, we cannot dodge this with a throwaway. **This is now the highest-severity risk in the project and it is stack-independent.** See §6. |

**~60 % of the project's total risk sits in Group A and is untouchable by technology choice.** That alone largely answers the question.

### Group B — Architecture-dependent. **Fixable without changing language.**

| Risk | The real lever |
|---|---|
| R3 Opus decode incomplete in Symphonia | *Who decodes* — not *what language*. Delegate to ffmpeg and the risk is gone. |
| R10 Stream resolution fragility | *Whether we have a fallback resolver* — not *what language*. |

### Group C — Genuinely language-dependent.

| Factor | Assessment |
|---|---|
| Completeness of the available YTM client library | **Python wins clearly.** See §3. |
| Speed at which upstream absorbs Google changes for us | **TS/Python win.** See §3. |
| Visualiser fidelity & CPU headroom | **Rust wins clearly.** See §4. |
| Audio-path reliability (no glitches) | **Rust wins.** |
| Single-binary distribution | **Rust/Go win.** |

So the decision reduces to: *is the Group-C library advantage of Python/TS worth the Group-C visualiser and audio disadvantage?*

---

## 2. Candidate stacks evaluated

| | Stack |
|---|---|
| **A** | Rust + ratatui + rustypipe (reads) + ytmusicapi-rs (writes) + symphonia/rodio — *the original plan* |
| **B** | Rust + ratatui + **pluggable resolver** (rustypipe → yt-dlp fallback) + **ffmpeg-piped decode** + in-house InnerTube write module — *the revision* |
| **C** | Python + Textual + ytmusicapi + yt-dlp + ffmpeg pipe + numpy/sounddevice |
| **D** | TypeScript + Ink + YouTube.js + mpv/ffmpeg sidecar |
| **E** | Go + bubbletea + innertube-go + beep/oto |
| **F** | Hybrid: Rust TUI + Python ytmusicapi sidecar over stdio JSON-RPC |

---

## 3. The library landscape — where other stacks genuinely beat Rust

### Python: `ytmusicapi` is the reference implementation

`ytmusicapi` (1.12.x) is the library every other YTM client is ported *from*. It covers the full surface in one package: search, home, explore, library, playlists (create/edit/reorder), history, lyrics, uploads, podcasts, and crucially **`rate_song`, `rate_playlist`, `edit_song_library_status`, `subscribe_artists`** — the exact write operations Rust makes us assemble from two crates.

Compare with Rust today:
- `rustypipe` 0.11 — excellent, actively maintained, best-in-class reads… but **the API is read-only.** No rating, no library edit, no playlist edit.
- `ytmusicapi` (Rust crate) 0.5.0 — has writes, but no search, single maintainer, pre-1.0.

**So Rust forces a two-library split with a seam right through our headline feature (like/dislike). Python has one library that does all of it.** That is a real, concrete advantage, and it is the strongest argument in the whole analysis for switching.

Second Python advantage: **yt-dlp lives in the same ecosystem.** No subprocess, no JSON parsing across a process boundary — `import yt_dlp`. When Google breaks something, you `pip install -U yt-dlp` and you're fixed, often within hours.

### TypeScript: `YouTube.js` is the most complete InnerTube client anywhere

`youtubei.js` (v18.x, released weeks ago, 120 dependents) is the most thorough InnerTube implementation in any language, with a dedicated YouTube Music namespace. And the decisive detail: **its author (LuanRT) also wrote the BotGuard interfacing library that every PO-token provider — including yt-dlp's — is built on.** PO-token fixes structurally land in the TS ecosystem *first*, and everyone else ports them.

If R2 (bot detection) is what you fear most, TS is objectively the closest to the source of fixes.

### Go: not competitive

The available InnerTube clients for Go are thin, low-level request wrappers with no YTM-parity parsing and no write operations. `beep`/`oto` cover mp3/flac/vorbis/wav — **no AAC, no Opus**, i.e. exactly the two formats YouTube serves. Go would mean building the API client *and* the codec path from scratch, with a TUI framework (bubbletea) that is good but weaker than ratatui for dense canvas rendering. **Strictly worse than Rust here. Eliminated.**

---

## 4. Where Rust wins — and why it happens to be where the product lives

### The visualiser needs an in-process PCM tap

The requirements doc specifies a tap that copies samples into a lock-free ring *as they pass to the mixer*. This is what makes the spectrum exact, perfectly synchronised, and zero-cost. It requires that **we own the decode → output path in our own process.**

The moment you delegate playback to an external player (mpv, ffplay), that tap is gone. Your options become:

| Out-of-process capture | Problem |
|---|---|
| PipeWire/PulseAudio **monitor source** (how `cava` does it) | Linux-only; captures *system* audio, so notification beeps and other apps pollute your spectrum unless you build a dedicated sink; breaks entirely on macOS/Windows |
| **FIFO** PCM tee (how MPD feeds cava) | mpv's `--ao=pcm` is a *sole* output — you cannot easily have real playback and a FIFO tee simultaneously; requires a filter-graph hack |
| mpv `--lavfi-complex` `showspectrum` | Renders a *video*, useless in a terminal |

**Conclusion: delegating playback to mpv would compromise the single feature that justifies the project.** This eliminates stack D as designed (Node has no credible in-process audio output path — it must shell out) and pushes stack C toward the ffmpeg-pipe design rather than python-mpv.

### CPU headroom is not theoretical here

Budget from the requirements doc: <3 % of one core for a 2048-point FFT at 60 fps *plus* per-frame band mapping, smoothing, peak tracking, and ~100 bars of sub-cell glyph rendering.

- Rust: comfortable, with room for the 4096-point "high detail" mode and the spectrogram/waterfall extra.
- Python: the FFT itself is a non-issue (numpy is C). The concern is the *per-frame Python work* — band mapping, smoothing, and driving Textual's diff/compositor at 60 fps — plus the GIL sharing a process with a realtime `sounddevice` callback. Achievable, but the budget goes from "comfortable" to "needs care," and stack C's P3 persona (Raspberry Pi) becomes doubtful.

### Rendering primitives

ratatui 0.30 shipped `Canvas` markers at **octant (2×4, densely packed), sextant (2×3), braille (2×4), half-block** — the octant marker is essentially purpose-built for exactly the bar rendering we want, and it landed this cycle. Textual/Rich gives you half-blocks natively and braille via third-party plotting widgets; Ink gives you very little. **This is a real, if modest, edge to Rust on the differentiating feature.**

### Distribution

Single static-ish binary (`cargo binstall`, AUR, brew) vs. Python's venv/PyInstaller reality or Node's `node_modules`. For a terminal tool people install on servers and Pis, this matters more than usual.

---

## 5. The finding that actually matters: change the architecture, not the language

Both Group-B risks dissolve with one design change, available in *any* language:

> **Pipe through ffmpeg for decode, and make stream resolution pluggable with yt-dlp as a fallback.**

```
resolver (trait)                    decoder                      us
┌──────────────────────┐
│ 1. rustypipe native  │──► stream URL ──► ffmpeg -i URL      ──► PCM f32
│ 2. yt-dlp -g (fallb.)│                   -f f32le -ar 48000     │
│ 3. (future impls)    │                   -ac 2 -               ├─► SPSC ring ─► analyser ─► spectrum
└──────────────────────┘                        (stdout pipe)     └─► rodio raw source ─► cpal ─► device
```

What this buys:

| Risk | Before | After |
|---|---|---|
| **R3 codec** | Opus decode incomplete in Symphonia; forced to AAC 128k; silence if we pick the wrong itag | **Eliminated.** ffmpeg decodes Opus, AAC, everything. We can take itag 251 (~160 kbps Opus) for better quality than the original plan allowed. |
| **R10 / R2 stream resolution** | Single in-house path; if it breaks, product is dead until *we* fix it | **Hugely reduced.** yt-dlp is the most battle-tested YouTube extractor in existence, updated within hours of breakage, by people who do nothing else. A `ytmtui --resolver=yt-dlp` flag turns a dead product into a `pip install -U` away from working. |
| **Visualiser tap** | Perfect | **Still perfect** — we still own the PCM in our process. This is the crucial difference from delegating to mpv. |
| **R5 audio-thread safety** | Good | Unchanged. |

Cost: a runtime dependency on `ffmpeg` (and optionally `yt-dlp`) on `PATH`. For a Linux terminal audience this is close to free, and `ytmtui doctor` can detect and explain it. Keep the pure-Rust symphonia AAC path as the zero-dependency default so `cargo install` still works standalone; ffmpeg becomes the *recommended* path.

The remaining Rust-specific weakness — the read/write library split — is fixable with a **~400-line in-house `innertube-write` module**. `like/like`, `like/dislike`, `like/removelike` and the feedback-token library calls are simple authenticated POSTs with a SAPISIDHASH header; `ytmusicapi`'s Python source is a readable reference implementation to port from. This is a bounded, one-time cost that removes a dependency on a pre-1.0 single-maintainer crate.

---

## 6. New top risk: R11, account safety

This came out of the research and belongs in the main risk register regardless of stack.

- OAuth no longer works for this class of access; **cookies are the only route**, and cookie-based third-party access carries a documented, non-trivial risk of temporary or permanent account restriction.
- We cannot use a throwaway account, because the entire point of FR-R1/R2 is that likes land on the **real** account and show up on your phone.

**Mitigations (all behavioural, all stack-independent) — promote these to requirements:**

1. **Look like a music player, not a scraper.** No bulk prefetch, no speculative downloading of whole playlists, no parallel request storms. One stream at a time, plus at most one prefetch of the *next* track.
2. **Cache aggressively** so repeat views don't re-hit the API (already FR-C2/C3 — now safety-critical, not just a performance feature).
3. **Rate-limit and back off** globally; never retry-loop a failure.
4. **No download/export feature.** Keeps the traffic pattern squarely "playback," which is also the honest description of what we're doing.
5. **Document the risk in the README** so the user opts in knowingly.
6. Consider a `--read-only` mode that never writes ratings, for users who don't want to authenticate at all.

---

## 7. Scored comparison

Weighted 1–5, weights reflect impact on *this* product (visualiser and API writes are the headline features).

| Criterion | Wt | **A** Rust orig. | **B** Rust rev. | **C** Python | **D** TS | **E** Go | **F** Hybrid |
|---|:--:|:--:|:--:|:--:|:--:|:--:|:--:|
| YTM API coverage incl. writes | 5 | 3 | 4 | **5** | **5** | 1 | **5** |
| Stream resolution robustness | 5 | 3 | **5** | **5** | **5** | 2 | **5** |
| Codec / decode risk | 4 | 2 | **5** | **5** | 4 | 3 | **5** |
| **Visualiser fidelity + headroom** | 5 | **5** | **5** | 3 | 2 | 4 | **5** |
| Audio playback reliability | 4 | 4 | 4 | 3 | 2 | 3 | 4 |
| TUI framework capability | 4 | **5** | **5** | 4 | 2 | 4 | **5** |
| Distribution / packaging | 3 | **5** | 4 | 2 | 3 | **5** | 2 |
| Maintenance when Google shifts | 4 | 3 | 4 | **5** | **5** | 2 | 4 |
| Fit to stated preference | 3 | **5** | **5** | 2 | 1 | 2 | 4 |
| **Weighted total (max 185)** | | 141 | **169** | 145 | 124 | 104 | 165 |
| **Score** | | 76 % | **91 %** | 78 % | 67 % | 56 % | 89 % |

**Reading the table:**

- **B wins outright.** It keeps every Rust advantage and imports the two things other ecosystems were winning on (yt-dlp's resolution robustness, ffmpeg's codec coverage) via subprocess boundaries rather than a rewrite.
- **F (Rust + Python sidecar) is nearly as good** and has the best raw capability, but it pays for it in distribution: you now ship a Rust binary *and* require a managed Python environment — the worst of both worlds for a `cargo binstall`-able terminal tool. **Keep F in the back pocket**: if the in-house write module proves painful, a narrow Python sidecar *only* for mutations is a legitimate escape hatch.
- **C (Python) is not a bad choice** — it's the fastest path to a working player, and if the visualiser were a nice-to-have I would probably recommend it. But the visualiser isn't a nice-to-have; it's the reason the project exists, and Python is where that feature gets hardest, not easiest.
- **D (TypeScript)** has the best API library and the fastest PO-token fixes, and loses anyway: no in-process audio output means no clean PCM tap, and Ink is the weakest canvas of the three.
- **E (Go)** loses on every axis that matters. Eliminated.

---

## 8. Recommendation

**Stay with Rust + Ratatui. Adopt stack B.** Concretely, three changes to the requirements doc:

1. **§4.4 / FR-P — decode strategy.** Default to an **ffmpeg-piped PCM decoder** (unlocking Opus itag 251), with the pure-Rust symphonia AAC path retained as a zero-dependency fallback. Feature-flag both; `doctor` reports which is active.
2. **§9.3 — `StreamResolver` gains a yt-dlp implementation from day one**, not "later." It is the single cheapest insurance policy in the project. Selectable via config and auto-failover on repeated 403s.
3. **§6.7 — write path is in-house.** Add a small `innertube-write` module (SAPISIDHASH + the like/dislike/library endpoints) instead of depending on the pre-1.0 `ytmusicapi` Rust crate. Port semantics from the Python `ytmusicapi` source, which is the de-facto spec.

Plus: **add R11 (account safety) to the risk register** with the behavioural mitigations in §6 promoted to requirements.

### Net effect on the risk profile

| Risk | Original | With stack B |
|---|---|---|
| R2 Stream access blocked | Medium-High → **product dead** | Medium → **degraded, recoverable via yt-dlp** |
| R3 Codec/Opus | High → silence | **Eliminated** |
| R7 GPL-3.0 propagation | Certain | Unchanged (still using rustypipe; in-house writes don't help here) |
| R11 Account ban | *not identified* | **High severity, mitigated behaviourally** |
| Everything else | — | Unchanged |

The M0 spike in §13 becomes even more valuable: it should now explicitly test **both** resolver implementations and **both** decode paths, so the go/no-go is made against the fallbacks, not just the happy path.

### What would change this recommendation

- If the M0 spike shows **stream resolution failing even with yt-dlp** on your IP/account → the problem is R2/R11, no stack fixes it, and the project should be reconsidered rather than rewritten.
- If you decide **the visualiser is optional** after all → switch to Python (stack C) without hesitation; it is the faster, lower-maintenance path to a good player.
- If the **in-house write module turns out to be a slog** → fall back to stack F's narrow Python sidecar for mutations only, or the `ytmusicapi` Rust crate, both contained behind the `MusicBackend` trait.

Because everything sits behind `MusicBackend` and `StreamResolver`, all three of those pivots are contained changes rather than rewrites. **The trait boundaries are the real hedge — more so than the language choice.**

---

## 9. Sources

- [bgutil-ytdlp-pot-provider](https://github.com/Brainicism/bgutil-ytdlp-pot-provider) — PO token provider, current limitations · [Rust port](https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs)
- [yt-dlp — cookie auth & account ban risk](https://github.com/yt-dlp/yt-dlp/issues/15724) · [YouTube authentication wiki](https://deepwiki.com/yt-dlp/yt-dlp-wiki/3.2-youtube-authentication)
- [YouTube.js (youtubei.js)](https://github.com/LuanRT/YouTube.js) · [npm](https://www.npmjs.com/package/youtubei.js) · [docs](https://ytjs.dev/guide/getting-started)
- [ytmusicapi (Python) — library & rating reference](https://ytmusicapi.readthedocs.io/en/stable/reference/library.html)
- [rustypipe query reference (read-only API)](https://docs.rs/rustypipe/latest/rustypipe/client/struct.RustyPipeQuery.html)
- [Symphonia codec support](https://github.com/pdeljanov/Symphonia) · [rodio](https://github.com/RustAudio/rodio)
- [Ratatui v0.30 highlights — octant/sextant markers](https://ratatui.rs/highlights/v030/)
- [cava — audio input backends](https://github.com/karlstav/cava/blob/master/README.md)
- [mpv manual — JSON IPC](https://mpv.io/manual/stable/)
- [Textual — animation & performance](https://textual.textualize.io/blog/2024/12/12/algorithms-for-high-performance-terminal-apps/)
- [innertube-go](https://pkg.go.dev/github.com/nezbut/innertube-go) · [beep](https://github.com/faiface/beep)
