# M0 Spike — Findings

**Verdict: GO — now complete.** Every risk M0 was built to test came back green or better than the requirements doc assumed. The last item, the credentialed write path, was confirmed on 2026-09-05. One risk is eliminated outright, one is materially lower than estimated, and one new *performance* problem was found that changes an M1 requirement.

| | |
|---|---|
| Date | 2026-09-04 |
| Stack | B (revised Rust) — see [TECH-STACK-RISK-ANALYSIS.md](./TECH-STACK-RISK-ANALYSIS.md) |
| Code | `spike/` — throwaway, as designed; **promoted into `crates/` during M1 and deleted** (see [M1-STATUS.md](./M1-STATUS.md)) |
| Environment | Rust 1.98.1 (mise, project-scoped), ffmpeg n9.0.1, yt-dlp 2026.08.19, PipeWire 1.6.8, truecolor |

---

## 1. Results against the M0 acceptance criteria

| # | Test | Result |
|---|---|---|
| **b1** | Resolve a YouTube Music stream URL | ✅ **GREEN** — works with **no cookies and no PO token** |
| **b2** | Decode it, play it, tap the PCM | ✅ **GREEN** — Opus *and* AAC, tap numerically validated |
| **c** | Render a live spectrum in ratatui | ✅ **GREEN** — 118 bands @ ~119 fps, inside CPU budget |
| **a** | Authenticated read + write (like/dislike) | ✅ **GREEN** — confirmed on a real account, 2026-09-05 |

---

## 2. R2 (stream access) — better than assumed

`yt-dlp` resolves `music.youtube.com` streams on this machine **anonymously**, with no PO token and no BotGuard helper. The returned URL carries `c=VISIONOS`, i.e. yt-dlp is currently routing through a client that isn't subject to the web-client PO-token requirement.

Full audio-only ladder is available:

```
251  webm  opus  129k   <- selected (best)
250  webm  opus   61k
249  webm  opus   46k
140  m4a   aac    130k  <- pure-Rust fallback path
139  m4a   aac     49k
```

**Consequences**

- The requirements doc's assumption that PO tokens are mandatory is **not currently true via yt-dlp**. `rustypipe-botguard` is not needed on this path at all.
- This is a *moving target*, not a permanent win — it depends on a client exemption Google can close. It strengthens rather than weakens the argument for the pluggable `StreamResolver`: the value of yt-dlp is precisely that someone else chases these changes.
- **R2 downgraded** from "Medium-High → product dead" to "Medium → degraded, recoverable."

## 3. R3 (codec) — eliminated, and we get the better stream

Both paths decoded to *exactly* the expected byte count with empty stderr:

```
itag 251 (Opus/WebM)  -> 1,920,000 bytes of f32le @48k stereo for 5s  (exact)
itag 140 (AAC/MP4)    -> 1,920,000 bytes                              (exact)
```

Because ffmpeg handles Opus, we take **itag 251 at ~129 kbps** rather than being forced down to AAC 128k by Symphonia's incomplete Opus decoder. **The revised stack is higher audio quality than the original plan, not merely safer.**

## 4. The visualiser works — and is numerically correct

End-to-end chain proven live: `yt-dlp → ffmpeg → rodio/cpal → lock-free tap → analyser thread → ArcSwap → ratatui`.

**Correctness check.** Playing a sweep whose true level ffmpeg independently measured at `mean_volume: -6.0 dB`, the spike's own tap reported `rms = -6.0 dB`. The tap and analyser are numerically right, not merely plausible-looking. The sweep also rendered as a single travelling peak with a peak-hold decay trail behind it — exactly the expected behaviour.

**Measured performance** (release build, 120×30 terminal, 118 bands, live Opus stream):

| Metric | Measured | Budget (§7.6) | |
|---|---|---|---|
| UI frame rate | 118–119 fps | ≥60 | ✅ |
| `viz` process CPU | 2.6 → 3.8 % | <3 % visualiser | ⚠️ at/just over |
| `ffmpeg` CPU | 1.4–3.3 % | — | |
| **Total** | **~5.2 %** | **<6 % whole app** | ✅ |
| Tap throughput | 1.73 M samples / 16 s ≈ realtime | no drops | ✅ |

The `viz` figure includes rendering at ~119 fps, roughly double the 60 fps target — capping the frame rate as specified in FR-V7 should bring the visualiser comfortably under its individual budget.

**Ratatui 0.30 `Marker::Octant` confirmed present** in `ratatui-core 0.1.2`, validating §7.3. The spike currently renders with partial-block glyphs (`▁▂▃▄▅▆▇█`, 8 sub-levels per cell), which is the most font-safe option; octant is available when we want it.

## 5. Two real defects found — which is what M0 is for

**(a) ffmpeg's `-reconnect*` options are HTTP-only.** Passing them for a local file makes ffmpeg abort with `Option reconnect not found`. Local-file playback was completely broken.

**(b) `stderr` was `Stdio::null()`, so (a) failed silently** — the UI showed a perfectly healthy 99 fps window with an empty spectrum and no error anywhere. This is the worst possible failure mode and it would have been much more expensive to diagnose in M2.

Both fixed: HTTP options are now applied only to network inputs, and ffmpeg's stderr is drained on a thread into a ring the UI surfaces in its status bar.

> **Promote to a requirement:** no subprocess in this project may have its stderr discarded. Silent degradation is the specific failure mode this architecture is most prone to, since it delegates to two external binaries.

## 6. Analyser tuning — done from measurement, not taste

The first live render **saturated**: nearly every band pinned at full height, reading as a solid wall. Rather than eyeball it, the spike grew a `tune` binary that dumps the real distribution of band values over ~25 s of audio.

| Percentile | Before | After | |
|---|---|---|---|
| p10 | 0.372 | **0.186** | quiet bands now actually look quiet |
| p25 | 0.484 | **0.320** | |
| p50 | 0.592 | **0.454** | median no longer half-height-plus |
| p90 | 0.774 | **0.694** | |
| p99 | 0.933 | 0.936 | peaks preserved |
| >0.95 (clipped) | 0.7 % | 0.7 % | never the real problem |

Diagnosis: the 60 dB display window was too wide, bunching everything in the upper half. Fixes, now in `analyser.rs`:

- `DB_FLOOR` −60 → **−45 dB** (narrower, higher-contrast window)
- **`GAMMA = 1.4`** expanding the quiet end
- **`TILT_DB_PER_OCT = 1.0`** gentle HF lift against music's natural roll-off

The result shows genuine spectral structure — bass hump, mid peaks, HF roll-off — instead of a block. `tune` stays as the instrument for M1 polish.

## 7. New finding: resolution latency is a UX problem (not a risk, a requirement)

**`yt-dlp` resolution takes ~3.4 s.** That is fine for a fallback and unacceptable as the interactive path — nobody presses Enter and waits 3.4 s in silence for a song to start.

This does not change the stack decision, but it does add an M1 requirement the original doc lacked:

> **FR-P12 (new, M):** stream URLs must be resolved *ahead of need*. Resolve the next queue item during playback of the current one, cache resolved URLs until their `expire` timestamp, and show an explicit spinner whenever a cold resolve is unavoidable. The native (rustypipe) resolver should be primary precisely because it avoids process-spawn latency; yt-dlp is the safety net, not the default.

## 8. R11 (account safety) — confirmed 2026-09-05

The `auth` binary is written and compiles: cookie parsing (Netscape *and* raw header), SAPISIDHASH signing, an authenticated read (`account/account_menu`, `browse FEmusic_liked_videos`), and the write path (`like/like`, `like/dislike`, `like/removelike`).

It is **read-only by default**; the write test is opt-in via `--like <videoId>` and restores the previous state afterwards.

**Result:** confirmed against a real account. Authenticated read returned the
account name and the Liked Songs page; `like/like` placed the track at the top
of Liked Songs and `like/removelike` restored the previous state:

```
== write path (this mutates your real account) ==
  already liked before : false
  sent like for sWcLccMuCA8
  present in liked now : true (at the top)
  like removed again - previous state restored

  WRITE PATH CONFIRMED
```

So the in-house InnerTube write client works: SAPISIDHASH signing, cookie auth,
and the rating endpoints. **No third-party YouTube Music crate is needed for the
ratings surface.**

To reproduce:

> **Updated in M1:** this now lives in the workspace as the `authcheck` binary.

```bash
mkdir -p ~/.config/ytmtui
cp ~/Downloads/music.youtube.com_cookies.txt ~/.config/ytmtui/cookies.txt
# or: export YTM_COOKIE='<raw Cookie header value>'

./target/release/authcheck                      # read-only check
./target/release/authcheck --like dQw4w9WgXcQ   # write path, then undoes it
```

⚠️ Read §6 of the risk analysis first. This authenticates as **your real account**, which is the point (likes must sync to your phone) and also the risk. The spike makes a handful of requests, which is a normal player-shaped traffic pattern.

---

## 9. What M0 changes in the plan

| Doc | Change |
|---|---|
| Risk register | R2 downgraded to Medium/recoverable; R3 marked eliminated with evidence; R11 still the top risk |
| §4.3 | PO tokens are **not** currently required on the yt-dlp path — note as a moving target |
| §6.5 | **New FR-P12**: resolve-ahead + URL caching (from §7 above) |
| §9.2 | **New NFR**: no subprocess may discard stderr |
| §7.2 | Analyser constants replaced with the measured values in §6 |
| §7.6 | Frame-rate cap is load-bearing, not just a setting — uncapped rendering doubles visualiser CPU |

## 10. Recommended next step

M0 is complete except for the credentialed test. **M1 (skeleton player)** is now unblocked and the highest-value next move: promote `spike/` into the real `crates/` workspace layout from §9.1, keeping `ytm-viz` I/O-free so `tune` keeps working, and build search → play → queue on top of the proven chain.

Do the `auth` run first, though — it is five minutes of your time and it is the last thing standing between the plan and a fully evidenced go/no-go.
