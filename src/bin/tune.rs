//! Analyser tuning instrument.
//!
//! Dumps the real distribution of band energies over a track so the display
//! mapping stays tuned from data rather than taste. The M0 constants in
//! `ytm-viz` came from this; re-run it after any change to them.
//!
//!   cargo run --release --bin tune -- <videoId | url | local file> [seconds]

use anyhow::Result;
use std::sync::Arc;
use ytm_core::VideoId;
use ytm_player::pcm::{self, FfmpegPcm, Progress};
use ytm_player::{ResolverCache, YtDlpResolver};
use ytm_viz::{Analyser, FFT_SIZE};

fn main() -> Result<()> {
    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dQw4w9WgXcQ".into());
    let seconds: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);

    let input = if std::path::Path::new(&arg).exists() {
        arg.clone()
    } else {
        let id = VideoId(extract_id(&arg));
        let r = ResolverCache::new(Arc::new(YtDlpResolver::default()));
        let f = r.resolve(&id)?;
        println!(
            "source: {} itag {} {} {:.0}k",
            id, f.itag, f.codec, f.abr_kbps
        );
        f.url
    };

    let (sink, mut tap) = pcm::tap(1 << 22);
    let mut src = FfmpegPcm::open(&input, 0.0, sink, Progress::default(), tap.errors())?;

    let n_bands = 96usize;
    let mut an = Analyser::new(n_bands, pcm::SAMPLE_RATE);
    let mut all: Vec<f32> = Vec::new();
    let mut per_band = vec![0.0f64; n_bands];
    let mut buf = Vec::with_capacity(1 << 16);
    let mut frames = 0usize;

    let hop = FFT_SIZE * pcm::CHANNELS as usize / 2;
    let target = pcm::SAMPLE_RATE as usize * pcm::CHANNELS as usize * seconds;
    let mut consumed = 0usize;

    while consumed < target {
        let mut got = 0;
        while got < hop {
            match src.next() {
                Some(_) => got += 1,
                None => break,
            }
        }
        if got == 0 {
            break;
        }
        consumed += got;
        buf.clear();
        tap.drain(&mut buf);
        if buf.is_empty() {
            continue;
        }
        an.feed_interleaved(&buf, pcm::CHANNELS as usize);
        let f = an.analyse(1.0 / 60.0);
        for (i, v) in f.bands.iter().enumerate() {
            per_band[i] += *v as f64;
            all.push(*v);
        }
        frames += 1;
    }

    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f32| {
        if all.is_empty() {
            0.0
        } else {
            all[((all.len() - 1) as f32 * p) as usize]
        }
    };

    println!("frames analysed : {frames}");
    println!("--- band value distribution (want a spread, not a pile at 1.0) ---");
    for p in [0.01, 0.10, 0.25, 0.50, 0.75, 0.90, 0.99] {
        println!("  p{:>3.0} = {:.3}", p * 100.0, pct(p));
    }
    let sat = all.iter().filter(|v| **v > 0.95).count() as f32 / all.len().max(1) as f32;
    println!("  saturated (>0.95) = {:.1}%", sat * 100.0);
    println!("--- mean per band, low -> high frequency ---");
    let g = n_bands / 16;
    for k in 0..16 {
        let m: f64 =
            per_band[k * g..(k + 1) * g].iter().sum::<f64>() / (g as f64 * frames.max(1) as f64);
        println!(
            "  {:>3}-{:>3}: {:.3} {}",
            k * g,
            (k + 1) * g - 1,
            m,
            "#".repeat((m * 40.0) as usize)
        );
    }
    Ok(())
}

fn extract_id(s: &str) -> String {
    if let Some(rest) = s.split("v=").nth(1) {
        return rest.chars().take(11).collect();
    }
    if s.contains("youtu.be/") {
        return s.rsplit('/').next().unwrap_or(s).chars().take(11).collect();
    }
    s.to_string()
}
