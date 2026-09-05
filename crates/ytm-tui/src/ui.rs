//! Rendering. A pure function of `App` plus the latest player status: no state
//! lives here, and nothing here performs I/O.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::Frame;
use std::sync::atomic::Ordering;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;
use ytm_api::SearchFilter;
use ytm_core::{fmt_duration, PlayerStatus, Row};

use crate::app::{App, Focus, Mode};
use crate::nav::Dest;
use crate::spectrum::Spectrum;

const MIN_W: u16 = 44;
const MIN_H: u16 = 10;
/// Below this the queue pane is dropped, then the sidebar (FR-U2).
const WIDE: u16 = 110;
const MEDIUM: u16 = 82;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_W || area.height < MIN_H {
        f.render_widget(
            Paragraph::new(format!(
                "terminal too small\n{}x{}, need {}x{}",
                area.width, area.height, MIN_W, MIN_H
            ))
            .alignment(Alignment::Center)
            .fg(app.theme.error),
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
        let [left, mid, right] = Layout::horizontal([
            Constraint::Length(20),
            Constraint::Min(30),
            Constraint::Length(34),
        ])
        .areas(body);
        draw_sidebar(f, app, left);
        draw_content(f, app, &status, mid);
        draw_queue(f, app, &status, right);
    } else if area.width >= MEDIUM {
        let [left, mid] =
            Layout::horizontal([Constraint::Length(18), Constraint::Min(30)]).areas(body);
        draw_sidebar(f, app, left);
        if app.focus == Focus::Queue {
            draw_queue(f, app, &status, mid);
        } else {
            draw_content(f, app, &status, mid);
        }
    } else {
        match app.focus {
            Focus::Sidebar => draw_sidebar(f, app, body),
            Focus::Queue => draw_queue(f, app, &status, body),
            Focus::Content => draw_content(f, app, &status, body),
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

fn spectrum_height(total: u16) -> u16 {
    match total {
        0..=18 => 4,
        19..=28 => 6,
        29..=40 => 9,
        _ => (total / 4).min(14),
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
        // Breadcrumb, so it is obvious how deep the navigation went.
        let mut spans = vec![
            Span::styled("ytmtui", Style::default().fg(app.theme.accent).bold()),
            Span::styled("  ", Style::default()),
        ];
        for (i, p) in app.stack.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" \u{203A} ", Style::default().fg(app.theme.dim)));
            }
            let last = i + 1 == app.stack.len();
            spans.push(Span::styled(
                p.view.title(),
                Style::default().fg(if last { app.theme.fg } else { app.theme.dim }),
            ));
        }
        if !app.backend.is_authenticated() {
            spans.push(Span::styled(
                "   (anonymous)",
                Style::default().fg(app.theme.dim),
            ));
        }
        Line::from(spans)
    };
    f.render_widget(
        Paragraph::new(line).block(block(app, "ytmtui".into(), app.mode == Mode::Search)),
        area,
    );
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Sidebar;
    let b = block(app, "browse".into(), focused);
    let inner = b.inner(area);
    f.render_widget(b, area);

    let items: Vec<ListItem> = app
        .sidebar
        .iter()
        .map(|d| match d {
            Dest::Separator(s) => ListItem::new(Line::from(Span::styled(
                format!("{}", s.to_uppercase()),
                Style::default().fg(app.theme.dim).add_modifier(Modifier::DIM),
            ))),
            Dest::Go(v) => ListItem::new(Line::from(Span::styled(
                format!("  {}", v.title()),
                Style::default().fg(app.theme.fg),
            ))),
        })
        .collect();

    let mut st = ListState::default();
    st.select(Some(app.sidebar_sel));
    f.render_stateful_widget(
        List::new(items).highlight_style(
            Style::default().bg(app.theme.selection_bg).add_modifier(Modifier::BOLD),
        ),
        inner,
        &mut st,
    );
}

fn row_item(app: &App, r: &Row, playing: bool, width: u16) -> ListItem<'static> {
    if let Row::Header(h) = r {
        return ListItem::new(Line::from(Span::styled(
            format!("{h}"),
            Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD),
        )));
    }
    let tag = r.tag();
    // marker(2) + title + gap(1) + subtitle + gap(1) + tag column(TAG_W).
    // Getting this wrong truncates the right-hand column, which is how "album"
    // renders as "alb" and "4:54" as "4".
    const TAG_W: usize = 9;
    let avail = (width as usize).saturating_sub(2 + 1 + 1 + TAG_W);
    let title_w = (avail * 6 / 10).max(8).min(avail);
    let sub_w = avail.saturating_sub(title_w);
    let marker = if playing { "\u{25B6} " } else { "  " };

    ListItem::new(Line::from(vec![
        Span::styled(marker, Style::default().fg(app.theme.accent)),
        Span::styled(
            fit(r.title(), title_w),
            Style::default().fg(if playing { app.theme.accent } else { app.theme.fg }),
        ),
        Span::styled(format!(" {}", fit(&r.subtitle(), sub_w)), Style::default().fg(app.theme.dim)),
        Span::styled(format!(" {:>w$}", tag, w = TAG_W), Style::default().fg(app.theme.dim)),
    ]))
}

fn draw_content(f: &mut Frame, app: &App, status: &PlayerStatus, area: Rect) {
    let focused = app.focus == Focus::Content;
    let Some(page) = app.page() else {
        f.render_widget(block(app, "\u{2026}".into(), focused), area);
        return;
    };

    let mut title = page.view.title();
    if page.loading {
        title.push_str(" \u{2026}");
    }
    // Search results carry tabs; show which one is active.
    if let Some(active) = page.view.filter() {
        let tabs: Vec<String> = SearchFilter::ALL
            .iter()
            .map(|f| {
                if *f == active {
                    format!("[{}]", f.label())
                } else {
                    f.label().to_string()
                }
            })
            .collect();
        title = format!("{}   {}", page.view.title().split(" \u{2022} ").next().unwrap_or(""), tabs.join(" "));
    }

    let b = block(app, title, focused);
    let inner = b.inner(area);
    f.render_widget(b, area);

    if page.rows.is_empty() {
        let msg = if page.loading {
            "loading\u{2026}".to_string()
        } else {
            page.error.clone().unwrap_or_else(|| "nothing here".into())
        };
        f.render_widget(Paragraph::new(msg).fg(app.theme.dim).wrap(Wrap { trim: true }), inner);
        return;
    }

    let now_id = status.current.as_ref().map(|t| t.id.clone());
    let items: Vec<ListItem> = page
        .rows
        .iter()
        .map(|r| {
            let playing = matches!(r, Row::Track(t) if Some(&t.id) == now_id.as_ref());
            row_item(app, r, playing, inner.width)
        })
        .collect();

    let mut st = ListState::default();
    st.select(Some(page.sel.min(page.rows.len() - 1)));
    f.render_stateful_widget(
        List::new(items).highlight_style(
            Style::default().bg(app.theme.selection_bg).add_modifier(Modifier::BOLD),
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
        .map(|(i, t)| row_item(app, &Row::Track(t.clone()), i == status.queue_index, inner.width))
        .collect();

    let mut st = ListState::default();
    st.select(Some(app.queue_sel.min(status.queue.len() - 1)));
    f.render_stateful_widget(
        List::new(items).highlight_style(
            Style::default().bg(app.theme.selection_bg).add_modifier(Modifier::BOLD),
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

    let flags = format!(
        "{}{}{}  {}{}  vol {:>3.0}%",
        if app.autoplay { "" } else { "autoplay:off " },
        if status.shuffle { "shuffle " } else { "" },
        status.repeat.glyph(),
        app.now.rating.glyph(),
        if app.now.in_library { " \u{2713}lib" } else { "" },
        status.volume * 100.0,
    );
    let name_w = top.width.saturating_sub(flags.width() as u16 + 1) as usize;

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{} ", status.state.glyph()), Style::default().fg(app.theme.accent)),
            Span::styled(
                truncate(&title, name_w.saturating_sub(artist.width() + 6)),
                Style::default().fg(app.theme.fg).bold(),
            ),
            Span::styled(
                if artist.is_empty() { String::new() } else { format!("  \u{2022}  {artist}") },
                Style::default().fg(app.theme.dim),
            ),
        ])),
        top,
    );
    f.render_widget(
        Paragraph::new(flags).alignment(Alignment::Right).fg(app.theme.dim),
        top,
    );

    if bar.height > 0 {
        let pos = status.position;
        let total = status.current.as_ref().and_then(|t| t.duration);
        let left = fmt_duration(pos);
        let right = total.map(fmt_duration).unwrap_or_else(|| "--:--".into());
        let track_w =
            bar.width.saturating_sub(left.width() as u16 + right.width() as u16 + 4) as usize;
        let frac = match total {
            Some(t) if t.as_secs_f64() > 0.0 => (pos.as_secs_f64() / t.as_secs_f64()).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let filled = (frac * track_w as f64) as usize;
        let mut track = String::new();
        for i in 0..track_w {
            track.push(if i + 1 < filled {
                '\u{2501}'
            } else if i + 1 == filled || (filled == 0 && i == 0) {
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
            " / search  Enter open  Esc back  Tab pane  space play  n/p  \u{2190}/\u{2192} seek  + like  a library  R radio  ? help  q quit"
                .to_string(),
            Style::default().fg(app.theme.dim),
        )
    };
    f.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_toast(f: &mut Frame, app: &App, area: Rect) {
    let Some((msg, _)) = &app.toast else { return };
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
        ("Enter", "open: play a track, or descend into album/artist/playlist"),
        ("Esc", "back"),
        ("Tab", "cycle sidebar / content / queue"),
        ("[ ]", "previous / next search tab"),
        ("g / G", "go to artist / album of the selection"),
        ("o / e", "play next / add to end of queue"),
        ("x", "remove from queue"),
        ("Space", "play / pause"),
        ("n / p", "next / previous (previous restarts if >3s in)"),
        ("\u{2190} \u{2192}", "seek 5s   (with Shift: 30s)"),
        ("9 / 0", "volume down / up"),
        ("+ or l", "thumbs up (toggles)"),
        ("- or d", "thumbs down (toggles, then skips)"),
        ("a", "add to / remove from library (not the same as a like)"),
        ("R / A", "radio from selection / toggle autoplay"),
        ("s / r", "shuffle / repeat mode"),
        ("v / z", "cycle visualiser style / fullscreen"),
        ("? / q", "this help / quit"),
    ];
    let w = 72.min(area.width.saturating_sub(4));
    let h = (rows.len() as u16 + 2).min(area.height.saturating_sub(2));
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
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

/// Truncate to a *display* width; counting chars misaligns every list
/// containing CJK or emoji, which real results routinely do.
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

fn fit(s: &str, w: usize) -> String {
    let t = truncate(s, w);
    let pad = w.saturating_sub(t.width());
    t + &" ".repeat(pad)
}
