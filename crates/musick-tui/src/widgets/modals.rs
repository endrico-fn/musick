use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use musick_core::{Track, UserPlaylist};

use crate::text::{split_row_cells, trunc_cells, wrap_cells};
use crate::theme::{DIM, TEXT, WARN_GOLD, active_theme, panel_block};
use crate::widgets::{BrailleSpinner, TextInput, render_scrollbar};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuAction {
    PlayNow,
    ShufflePlay,
    PlayNext,
    AddToQueue,
    StartRadio,
    GoArtist,
    GoAlbum,
    AddToPlaylist,
    SaveCopyToMyPlaylists,
    ToggleLike,
    ToggleSave,
    ToggleArtistFollow,
    ShowArtistBio,
    CopyUrl,
    RemoveItem,
}

#[derive(Debug, Clone)]
pub struct ContextMenuItem {
    pub key: &'static str,
    pub label: String,
    pub action: ContextMenuAction,
}

#[derive(Debug, Clone)]
pub enum ActiveModal {
    CreatePlaylist {
        input: TextInput,
    },
    RenamePlaylist {
        playlist_id: String,
        input: TextInput,
    },
    ImportPlaylistUrl {
        input: TextInput,
    },
    AddToPlaylist {
        track: Track,
        playlists: Vec<UserPlaylist>,
        cursor: usize,
    },
    Confirm {
        title: String,
        message: String,
        action: ConfirmAction,
    },
    OAuthWizard {
        client_id: String,
        client_secret: String,
        page_id: String,
        active_field: usize,
        device_user_code: Option<String>,
        verification_url: Option<String>,
        polling: bool,
        error: Option<String>,
    },
    BrowserAuthImport {
        raw_headers: String,
        page_id: String,
        active_field: usize,
        error: Option<String>,
    },
    TextViewer {
        title: String,
        body: String,
        scroll: usize,
    },
    ContextMenu {
        title: String,
        subtitle: String,
        items: Vec<ContextMenuItem>,
        cursor: usize,
    },
    KeymapHelp {
        scroll: usize,
    },
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    DeletePlaylist { playlist_id: String },
    ClearCache,
    Logout,
}

pub fn centered_rect(width_pct: u16, height: u16, area: Rect) -> Rect {
    let w = ((u32::from(area.width) * u32::from(width_pct)) / 100)
        .clamp(36, u32::from(area.width.saturating_sub(2)).max(1)) as u16;
    let h = height.min(area.height.saturating_sub(2)).max(5);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

pub fn render_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    modal: &ActiveModal,
    spinner: BrailleSpinner,
) {
    if area.width < 24 || area.height < 8 {
        return;
    }

    match modal {
        ActiveModal::CreatePlaylist { input } => {
            render_text_prompt(
                frame,
                area,
                "Create Playlist",
                "Playlist name:",
                input,
                "Enter to create · Esc to cancel",
            );
        }
        ActiveModal::RenamePlaylist { input, .. } => {
            render_text_prompt(
                frame,
                area,
                "Rename Playlist",
                "New playlist name:",
                input,
                "Enter to rename · Esc to cancel",
            );
        }
        ActiveModal::ImportPlaylistUrl { input } => {
            render_text_prompt(
                frame,
                area,
                "Import Playlist",
                "YouTube Music URL, playlist ID, or local .json/.m3u8 path:",
                input,
                "Enter to import · Esc to cancel",
            );
        }
        ActiveModal::AddToPlaylist {
            track,
            playlists,
            cursor,
        } => {
            let rect = centered_rect(64, (playlists.len() as u16 + 7).min(18), area);
            frame.render_widget(Clear, rect);
            let block = panel_block("Add to Playlist", Some("Enter select · Esc cancel"), true);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);

            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Track: ", Style::default().fg(DIM).bg(Color::Reset)),
                    Span::styled(
                        trunc_cells(&track.title, usize::from(inner.width.saturating_sub(8))),
                        Style::default()
                            .fg(TEXT)
                            .bg(Color::Reset)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
            ];

            if playlists.is_empty() {
                lines.push(Line::from(Span::styled(
                    "No playlists yet — switch to 2 Playlists and press c to create one.",
                    Style::default().fg(DIM).bg(Color::Reset),
                )));
            } else {
                let clamped = (*cursor).min(playlists.len().saturating_sub(1));
                let visible_h = usize::from(inner.height.saturating_sub(2)).max(1);
                let scroll_start = if clamped >= visible_h {
                    clamped + 1 - visible_h
                } else {
                    0
                };
                let row_w = usize::from(inner.width);
                let theme = active_theme();
                for (i, pl) in playlists
                    .iter()
                    .enumerate()
                    .skip(scroll_start)
                    .take(visible_h)
                {
                    let sel = i == clamped;
                    let prefix = if sel { "▸ " } else { "  " };
                    let bg = if sel { theme.cursor_bg } else { Color::Reset };
                    let label = format!("{prefix}{} ({} tracks)", pl.title, pl.track_count);
                    lines.push(Line::from(Span::styled(
                        trunc_cells(&label, row_w),
                        Style::default()
                            .fg(if sel { theme.accent } else { TEXT })
                            .bg(bg),
                    )));
                }
            }
            frame.render_widget(Paragraph::new(lines), inner);
        }
        ActiveModal::Confirm { title, message, .. } => {
            let rect = centered_rect(58, 9, area);
            frame.render_widget(Clear, rect);
            let block = panel_block(title, Some("Enter confirm · Esc cancel"), true);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);

            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    message.as_str(),
                    Style::default().fg(TEXT).bg(Color::Reset),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "[Enter] Confirm    [Esc] Cancel",
                    Style::default()
                        .fg(WARN_GOLD)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                )),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true }),
                inner,
            );
        }
        ActiveModal::OAuthWizard {
            client_id,
            client_secret,
            page_id,
            active_field,
            device_user_code,
            verification_url,
            polling,
            error,
        } => {
            let rect = centered_rect(72, 14, area);
            frame.render_widget(Clear, rect);
            let block = panel_block(
                "OAuth2 Device Login",
                Some("Tab field · Enter continue · Esc cancel"),
                true,
            );
            let inner = block.inner(rect);
            frame.render_widget(block, rect);

            let mut lines = Vec::new();
            if let (Some(code), Some(url)) = (device_user_code, verification_url) {
                lines.push(Line::from(Span::styled(
                    "Open this URL in your browser and enter the device code:",
                    Style::default().fg(TEXT).bg(Color::Reset),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  URL:  {url}"),
                    Style::default().fg(active_theme().accent).bg(Color::Reset),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  Code: {code}"),
                    Style::default()
                        .fg(WARN_GOLD)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                if *polling {
                    lines.push(spinner.loading_line("Waiting for browser authorization…"));
                }
            } else {
                lines.push(field_line(
                    "Client ID     ",
                    client_id,
                    *active_field == 0,
                    false,
                ));
                lines.push(field_line(
                    "Client Secret ",
                    client_secret,
                    *active_field == 1,
                    true,
                ));
                lines.push(field_line(
                    "Brand Page ID ",
                    page_id,
                    *active_field == 2,
                    false,
                ));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Leave Brand Page ID blank for primary account. Stored in OS Keyring.",
                    Style::default().fg(DIM).bg(Color::Reset),
                )));
            }
            if let Some(err) = error {
                lines.push(Line::from(Span::styled(
                    format!("Error: {err}"),
                    Style::default().fg(crate::theme::ERR).bg(Color::Reset),
                )));
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        ActiveModal::BrowserAuthImport {
            raw_headers,
            page_id,
            active_field,
            error,
        } => {
            let rect = centered_rect(76, 13, area);
            frame.render_widget(Clear, rect);
            let block = panel_block(
                "Browser Auth Import (SAPISIDHASH)",
                Some("Tab field · Enter save · Esc cancel"),
                true,
            );
            let inner = block.inner(rect);
            frame.render_widget(block, rect);

            let preview = if raw_headers.is_empty() {
                String::new()
            } else {
                format!("({} chars pasted)", raw_headers.len())
            };
            let mut lines = vec![
                Line::from(Span::styled(
                    "Paste Cookie header (containing SAPISID / __Secure-3PAPISID) or DevTools headers:",
                    Style::default().fg(DIM).bg(Color::Reset),
                )),
                field_line("Cookie/Headers ", &preview, *active_field == 0, false),
                field_line("Brand Page ID  ", page_id, *active_field == 1, false),
                Line::from(""),
                Line::from(Span::styled(
                    "Credentials are stored strictly in OS Keyring (0o600 fallback). Never sent to yt-dlp.",
                    Style::default().fg(DIM).bg(Color::Reset),
                )),
            ];
            if let Some(err) = error {
                lines.push(Line::from(Span::styled(
                    format!("Error: {err}"),
                    Style::default().fg(crate::theme::ERR).bg(Color::Reset),
                )));
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        ActiveModal::TextViewer {
            title,
            body,
            scroll,
        } => {
            let rect = centered_rect(76, 18, area);
            frame.render_widget(Clear, rect);
            let block = panel_block(title, Some("j/k scroll · Esc/i close"), true);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            if inner.width < 6 || inner.height == 0 {
                return;
            }
            let wrap_w = usize::from(inner.width.saturating_sub(2)).max(4);
            let mut wrapped_lines: Vec<String> = Vec::new();
            for para in body.split('\n') {
                let p = para.trim();
                if p.is_empty() {
                    if !wrapped_lines.is_empty() {
                        wrapped_lines.push(String::new());
                    }
                } else {
                    wrapped_lines.extend(wrap_cells(p, wrap_w));
                }
            }
            let vis_h = usize::from(inner.height);
            let total = wrapped_lines.len();
            let max_scroll = total.saturating_sub(vis_h);
            let clamped = (*scroll).min(max_scroll);
            let has_sb = total > vis_h && inner.width > 8;
            let text_w = if has_sb {
                inner.width.saturating_sub(2)
            } else {
                inner.width
            };
            let lines: Vec<Line<'_>> = wrapped_lines
                .iter()
                .skip(clamped)
                .take(vis_h)
                .map(|ln| {
                    Line::from(Span::styled(
                        ln.as_str(),
                        Style::default().fg(TEXT).bg(Color::Reset),
                    ))
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines),
                Rect {
                    x: inner.x,
                    y: inner.y,
                    width: text_w,
                    height: inner.height,
                },
            );
            if has_sb {
                render_scrollbar(
                    frame.buffer_mut(),
                    inner.x + inner.width - 1,
                    inner.y,
                    vis_h,
                    total,
                    clamped,
                    true,
                );
            }
        }
        ActiveModal::ContextMenu {
            title,
            subtitle,
            items,
            cursor,
        } => {
            let extra_h = if subtitle.is_empty() { 4 } else { 6 };
            let rect = centered_rect(54, (items.len() as u16 + extra_h).min(18), area);
            frame.render_widget(Clear, rect);
            let block = panel_block(title, Some("j/k move · Enter select · Esc close"), true);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            if inner.width < 8 || inner.height == 0 {
                return;
            }
            let theme = active_theme();
            let mut lines = Vec::new();
            let header_rows = if subtitle.is_empty() {
                0usize
            } else {
                lines.push(Line::from(Span::styled(
                    trunc_cells(subtitle, usize::from(inner.width)),
                    Style::default().fg(DIM).bg(Color::Reset),
                )));
                lines.push(Line::from(""));
                2usize
            };
            let clamped = (*cursor).min(items.len().saturating_sub(1));
            let visible_h = usize::from(inner.height).saturating_sub(header_rows).max(1);
            let scroll_start = if clamped >= visible_h {
                clamped + 1 - visible_h
            } else {
                0
            };
            let row_w = usize::from(inner.width);
            for (i, item) in items.iter().enumerate().skip(scroll_start).take(visible_h) {
                let sel = i == clamped;
                let bg = if sel { theme.cursor_bg } else { Color::Reset };
                let lead = if sel { "▸ " } else { "  " };
                let (left_s, right_s, gap) = split_row_cells(&item.label, item.key, row_w, 2, 0.3);
                let mut label_style = Style::default()
                    .fg(if sel { theme.accent } else { TEXT })
                    .bg(bg);
                if sel {
                    label_style = label_style.add_modifier(Modifier::BOLD);
                }
                let mut spans = vec![
                    Span::styled(
                        lead,
                        Style::default()
                            .fg(if sel { theme.accent } else { DIM })
                            .bg(bg),
                    ),
                    Span::styled(left_s, label_style),
                ];
                if !right_s.is_empty() {
                    spans.push(Span::styled(" ".repeat(gap), Style::default().bg(bg)));
                    spans.push(Span::styled(
                        right_s,
                        Style::default()
                            .fg(if sel { theme.accent } else { DIM })
                            .bg(bg),
                    ));
                }
                lines.push(Line::from(spans));
            }
            frame.render_widget(Paragraph::new(lines), inner);
        }
        ActiveModal::KeymapHelp { scroll } => {
            let rect = centered_rect(84, 24, area);
            frame.render_widget(Clear, rect);
            let block = panel_block(
                "Keymap & Controls Reference",
                Some("j/k scroll · Esc/? close"),
                true,
            );
            let inner = block.inner(rect);
            frame.render_widget(block, rect);

            let entries: &[(&str, &str)] = &[
                (
                    "1 - 5 / gt gT",
                    "Switch root tabs (Player · Playlists · Library · Queue · Settings)",
                ),
                (
                    "Tab / Shift+Tab",
                    "Next / previous sub-tab (Discover, Library, Artist, Settings)",
                ),
                (
                    "Ctrl+w + h/j/k/l",
                    "Focus split panes (Ctrl+w j focuses Now Playing controls/seekbar)",
                ),
                (
                    "Mouse",
                    "Click/drag progress bar, volume, buttons, tabs · Right-click menu",
                ),
                (
                    "h j k l / Arrows",
                    "Move cursor (or scrub seekbar/controls when bar focused)",
                ),
                ("gg / G", "Jump to top / bottom"),
                ("Ctrl+d / Ctrl+u", "Half-page scroll down / up"),
                (
                    "A / a  (or ga/gl)",
                    "Jump to hovered or playing Artist / Album (Artist: a opens Albums)",
                ),
                (
                    "gA / gL",
                    "Always jump to currently playing track's Artist / Album",
                ),
                (
                    "e / Right-click",
                    "Open context action menu for selected item",
                ),
                ("gr / g/", "Refresh Discover feed / Global search"),
                ("L (Shift+L)", "Toggle expanded Lyrics globally"),
                ("zz / 0", "Recenter Lyrics on active line"),
                (", / .", "Shift Lyrics timing -0.25s / +0.25s"),
                ("t", "Toggle Indonesian lyrics translation"),
                (
                    "S (Shift+S)",
                    "Sync playlists & library with YouTube Music now",
                ),
                ("/", "Search (Player) or inline filter in active view"),
                ("Space / n / b", "Play-Pause / Next track / Previous track"),
                ("] [  or  } {", "Seek +5s / -5s  or  +30s / -30s"),
                ("+ - / M", "Volume +5% / -5% (0-130%) / Mute toggle"),
                ("s / m", "Toggle Shuffle / Cycle Repeat (off -> all -> one)"),
                ("Enter / p", "Play or Open / Play entity directly"),
                (
                    "N / q",
                    "Play Next (front of queue) / Add to Queue (FIFO priority)",
                ),
                (
                    "P / f / w / W",
                    "Add to Playlist / Like / Save item / Follow Artist",
                ),
                (
                    "r / x / i / o",
                    "Start Mix Radio / Shuffle Play / Artist Bio / Cycle Sort",
                ),
                ("c / dd / Del", "Create playlist / Remove or delete item"),
                (
                    "J / K (Shift)",
                    "Reorder track down / up (Queue & User Playlists)",
                ),
                (
                    "u / D / C / X",
                    "Undo queue / Deduplicate / Clear upcoming / Clear all",
                ),
                (
                    "R / I / E / U",
                    "Playlists: Rename / Import URL / Export JSON+M3U8 / Upload to YTM",
                ),
                (
                    "yy / Y / Bksp / Q",
                    "Copy item URL / Copy collection URL / Back / Quit (or Ctrl+C)",
                ),
            ];

            let accent = active_theme().accent;
            let max_scroll = entries.len().saturating_sub(usize::from(inner.height));
            let clamped_scroll = (*scroll).min(max_scroll);
            let lines: Vec<Line<'_>> = entries
                .iter()
                .skip(clamped_scroll)
                .map(|&(k, desc)| {
                    Line::from(vec![
                        Span::styled(
                            format!("{k:<18} "),
                            Style::default()
                                .fg(accent)
                                .bg(Color::Reset)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(desc, Style::default().fg(TEXT).bg(Color::Reset)),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }
}

fn field_line<'a>(label: &'a str, val: &str, focused: bool, mask: bool) -> Line<'a> {
    let theme = active_theme();
    let shown = if mask && !val.is_empty() {
        "•".repeat(val.chars().count().min(24))
    } else {
        val.to_string()
    };
    let cursor = if focused { "█" } else { "" };
    let bg = if focused {
        theme.cursor_bg
    } else {
        Color::Reset
    };
    Line::from(vec![
        Span::styled(label, Style::default().fg(DIM).bg(Color::Reset)),
        Span::styled(
            format!(" {shown}{cursor} "),
            Style::default()
                .fg(if focused { theme.accent } else { TEXT })
                .bg(bg),
        ),
    ])
}

fn render_text_prompt(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    prompt: &str,
    input: &TextInput,
    hint: &str,
) {
    let rect = centered_rect(64, 8, area);
    frame.render_widget(Clear, rect);
    let block = panel_block(title, Some(hint), true);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let text_budget = usize::from(inner.width.saturating_sub(3)).max(4);
    let mut input_spans = vec![Span::styled(
        "› ",
        Style::default().fg(active_theme().accent).bg(Color::Reset),
    )];
    input_spans.extend(input.render_spans(text_budget, true, ""));

    let lines = vec![
        Line::from(Span::styled(
            prompt,
            Style::default().fg(DIM).bg(Color::Reset),
        )),
        Line::from(""),
        Line::from(input_spans),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
