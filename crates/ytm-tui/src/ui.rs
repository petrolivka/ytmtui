//! Rendering. Pure function of `App` plus the latest player status: no state
//! lives here, and nothing in here performs I/O.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;
use std::sync::atomic::Ordering;
use ytm_core::{fmt_duration, PlayerStatus, Track};

use crate::app::{App, Focus, Mode};
use crate::spectrum::Spectrum;

/// Below this the layout stops being useful; say so rather than render garbage.
const MIN_W: u16 = 44;
const MIN_H: u16 = 10;
/// Queue panel is dropped below this width (FR-U2, progressive disclosure).
const WIDE: u16 = 92;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_W || area.height < MIN_H {
        let msg = format!(
            "terminal too small\n{}x{}, need {}x{}",
            area.width, area.height, MIN_W, MIN_H
        );
        f.render_widget(
            Paragraph::new(msg).alignment(Alignment::Center).fg(app.theme.error),
            area,
        );
        return;
    }

    let status = app.player.status();

    if app.viz_fullscreen {
        let [main, now] = Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).areas(area);
        draw_spectrum(f, app, main);
        draw_now_playing(f, app, &status, now);
        draw_toast(f, app, area);
        return;
    }

    let [header, body, viz, now, foot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(spectrum_height(area.height)),
        Constraint::Length(4),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header);

    if area.width >= WIDE {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(body);
        draw_results(f, app, left);
        draw_queue(f, app, &status, right);
    } else {
        match app.focus {
            Focus::Results => draw_results(f, app, body),
            Focus::Queue => draw_queue(f, app, &status, body),
        }
    }

    draw_spectrum(f, app, viz);
    draw_now_playing(f, app, &status, now);
    draw_status_bar(f, app, &status, foot);
    draw_toast(f, app, area);

    if app.show_help {
        draw_help(f, app, area);
    }
}

/// Give the spectrum a generous share of a tall terminal, but never starve the
/// lists on a short one.
fn spectrum_height(total: u16) -> u16 {
    match total {
        0..=18 => 4,
        19..=28 => 7,
        29..=40 => 10,
        _ => (total / 3).min(16),
    }
}

fn block<'a>(app: &App, title: String, focused: bool) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused {
            app.theme.border_focus
        } else {
            app.theme.border
        }))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(if focused { app.theme.fg } else { app.theme.dim }),
        ))
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let line = if app.mode == Mode::Search {
        Line::from(vec![
            Span::styled("/", Style::default().fg(app.theme.accent).bold()),
            Span::raw(app.query.clone()),
            Span::styled("\u{2588}", Style::default().fg(app.theme.accent)),
        ])
    } else {
        let auth = if app.backend.is_authenticated() {
            Span::styled("signed in", Style::default().fg(app.theme.ok))
        } else {
            Span::styled("anonymous", Style::default().fg(app.theme.dim))
        };
        Line::from(vec![
            Span::styled("ytmtui", Style::default().fg(app.theme.accent).bold()),
            Span::styled("  \u{2022}  ", Style::default().fg(app.theme.dim)),
            auth,
            Span::styled("  \u{2022}  press / to search, ? for help", Style::default().fg(app.theme.dim)),
        ])
    };
    f.render_widget(
        Paragraph::new(line).block(block(app, "ytmtui".into(), app.mode == Mode::Search)),
        area,
    );
}

fn track_line(app: &App, t: &Track, playing: bool, width: u16) -> ListItem<'static> {
    let dur = t.duration_str();
    // Reserve room for the marker, the gap and the duration column.
    let avail = width.saturating_sub(dur.len() as u16 + 6) as usize;
    let title_w = (avail * 6 / 10).max(8);
    let artist_w = avail.saturating_sub(title_w);

    let marker = if playing { "\u{25B6} " } else { "  " };

    ListItem::new(Line::from(vec![
        Span::styled(marker, Style::default().fg(app.theme.accent)),
        Span::styled(
            fit(&t.title, title_w),
            Style::default().fg(if playing { app.theme.accent } else { app.theme.fg }),
        ),
        Span::styled(format!(" {}", fit(&t.artist, artist_w)), Style::default().fg(app.theme.dim)),
        Span::styled(format!(" {dur:>5}"), Style::default().fg(app.theme.dim)),
    ]))
}

fn draw_results(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Results;
    let title = if app.searching {
        format!("{} \u{2026}", app.results_title)
    } else {
        format!("{} ({})", app.results_title, app.results.len())
    };
    let b = block(app, title, focused);
    let inner = b.inner(area);
    f.render_widget(b, area);

    if app.results.is_empty() {
        let msg = if app.searching { "loading\u{2026}" } else { "press / to search" };
        f.render_widget(Paragraph::new(msg).fg(app.theme.dim), inner);
        return;
    }

    let now_id = app.player.status().current.as_ref().map(|t| t.id.clone());
    let items: Vec<ListItem> = app
        .results
        .iter()
        .map(|t| track_line(app, t, Some(&t.id) == now_id.as_ref(), inner.width))
        .collect();

    let mut st = ListState::default();
    st.select(Some(app.results_sel));
    f.render_stateful_widget(
        List::new(items).highlight_style(
            Style::default()
                .bg(app.theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        ),
        inner,
        &mut st,
    );
}

fn draw_queue(f: &mut Frame, app: &App, status: &PlayerStatus, area: Rect) {
    let focused = app.focus == Focus::Queue;
    let b = block(app, format!("Queue ({})", status.queue.len()), focused);
    let inner = b.inner(area);
    f.render_widget(b, area);

    if status.queue.is_empty() {
        f.render_widget(Paragraph::new("empty").fg(app.theme.dim), inner);
        return;
    }
    let items: Vec<ListItem> = status
        .queue
        .iter()
        .enumerate()
        .map(|(i, t)| track_line(app, t, i == status.queue_index, inner.width))
        .collect();

    let mut st = ListState::default();
    st.select(Some(app.queue_sel.min(status.queue.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items).highlight_style(
            Style::default()
                .bg(app.theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        ),
        inner,
        &mut st,
    );
}

fn draw_spectrum(f: &mut Frame, app: &App, area: Rect) {
    let frame = app.spectrum.load_full();
    let b = block(
        app,
        format!("spectrum \u{2022} {} \u{2022} {} bands", app.viz_style.name(), frame.bands.len()),
        false,
    );
    let inner = b.inner(area);
    f.render_widget(b, area);
    // Give each band its own column plus a gutter when there is room; fall back
    // to contiguous bars on narrow terminals rather than showing too few bands.
    let step: u16 = if inner.width >= 60 { 2 } else { 1 };
    let bands = (inner.width / step).max(1);
    app.n_bands.store(bands as u64, Ordering::Relaxed);
    f.render_widget(
        Spectrum { frame: &frame, style: app.viz_style, theme: &app.theme, step },
        inner,
    );
}

fn draw_now_playing(f: &mut Frame, app: &App, status: &PlayerStatus, area: Rect) {
    let b = block(app, "now playing".into(), false);
    let inner = b.inner(area);
    f.render_widget(b, area);
    if inner.height == 0 {
        return;
    }

    let [top, bar] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    let (title, artist) = match &status.current {
        Some(t) => (t.title.clone(), t.artist.clone()),
        None => ("nothing playing".into(), String::new()),
    };
    let rating = status.current.as_ref().map(|t| t.rating).unwrap_or_default();

    let flags = format!(
        "{}{}  {}  vol {:>3.0}%",
        if status.shuffle { "shuffle " } else { "" },
        status.repeat.glyph(),
        rating.glyph(),
        status.volume * 100.0,
    );
    let flags_w = flags.len() as u16 + 1;
    let name_w = top.width.saturating_sub(flags_w) as usize;

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} ", status.state.glyph()),
                Style::default().fg(app.theme.accent),
            ),
            Span::styled(
                truncate(&title, name_w.saturating_sub(artist.len() + 6)),
                Style::default().fg(app.theme.fg).bold(),
            ),
            Span::styled(
                if artist.is_empty() { String::new() } else { format!("  \u{2022}  {artist}") },
                Style::default().fg(app.theme.dim),
            ),
        ]))
        .block(Block::default()),
        top,
    );
    f.render_widget(
        Paragraph::new(flags).alignment(Alignment::Right).fg(app.theme.dim),
        top,
    );

    // Progress bar drawn as text so it degrades cleanly at any width.
    if bar.height > 0 {
        let pos = status.position;
        let total = status.current.as_ref().and_then(|t| t.duration);
        let left = fmt_duration(pos);
        let right = total.map(fmt_duration).unwrap_or_else(|| "--:--".into());
        let track_w = bar.width.saturating_sub(left.len() as u16 + right.len() as u16 + 4) as usize;
        let frac = match total {
            Some(t) if t.as_secs_f64() > 0.0 => {
                (pos.as_secs_f64() / t.as_secs_f64()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        let filled = (frac * track_w as f64) as usize;
        let mut track = String::new();
        for i in 0..track_w {
            track.push(if i < filled.saturating_sub(1) {
                '\u{2501}'
            } else if i == filled.saturating_sub(1) || (filled == 0 && i == 0) {
                '\u{25CF}'
            } else {
                '\u{2500}'
            });
        }
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{left} "), Style::default().fg(app.theme.dim)),
                Span::styled(track, Style::default().fg(app.theme.accent)),
                Span::styled(format!(" {right}"), Style::default().fg(app.theme.dim)),
            ])),
            bar,
        );
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, status: &PlayerStatus, area: Rect) {
    let (text, style) = if let Some(err) = &status.error {
        (format!(" {err}"), Style::default().fg(app.theme.error))
    } else {
        (
            " space play/pause  n/p next/prev  \u{2190}/\u{2192} seek  9/0 vol  + like  - dislike  s shuffle  r repeat  v viz  z zoom  ? help  q quit"
                .to_string(),
            Style::default().fg(app.theme.dim),
        )
    };
    f.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_toast(f: &mut Frame, app: &App, area: Rect) {
    let Some((msg, _)) = &app.toast else { return };
    // Float above the status bar rather than over the header, so it never
    // obscures the search field or the pane titles.
    let w = (msg.width() as u16 + 4).min(area.width.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(w + 1);
    let y = area.y + area.height.saturating_sub(4);
    let r = Rect { x, y, width: w, height: 3 };
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(msg.clone())
            .fg(app.theme.fg)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(app.theme.accent)),
            )
            .wrap(Wrap { trim: true }),
        r,
    );
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let rows = [
        ("/", "search"),
        ("Enter", "play selection (results) / jump to (queue)"),
        ("o / e", "play next / add to end of queue"),
        ("x", "remove from queue"),
        ("Space", "play / pause"),
        ("n / p", "next / previous (previous restarts if >3s in)"),
        ("\u{2190} \u{2192}", "seek 5s   (with Shift: 30s)"),
        ("9 / 0", "volume down / up"),
        ("+ or l", "thumbs up (toggles)"),
        ("- or d", "thumbs down (toggles, then skips)"),
        ("s / r", "shuffle / repeat mode"),
        ("Tab", "switch pane"),
        ("v / z", "cycle visualiser style / fullscreen"),
        ("? / q", "this help / quit"),
    ];
    let w = 62.min(area.width.saturating_sub(4));
    let h = (rows.len() as u16 + 2).min(area.height.saturating_sub(2));
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("  {k:<8}"), Style::default().fg(app.theme.accent).bold()),
                Span::styled((*d).to_string(), Style::default().fg(app.theme.fg)),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(app.theme.border_focus))
                .title(" keys \u{2022} any key to close "),
        ),
        r,
    );
}

/// Truncate to a *display* width, appending an ellipsis when it does not fit.
/// Counting chars instead of columns misaligns every list containing CJK or
/// emoji, which real search results routinely do.
fn truncate(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    if s.width() <= w {
        return s.to_string();
    }
    if w == 1 {
        return "\u{2026}".into();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw > w - 1 {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('\u{2026}');
    out
}

/// Truncate then pad to exactly `w` display columns.
fn fit(s: &str, w: usize) -> String {
    let t = truncate(s, w);
    let pad = w.saturating_sub(t.width());
    t + &" ".repeat(pad)
}
