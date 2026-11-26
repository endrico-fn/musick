use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use musick_core::{Track, UserPlaylist};

use crate::text::trunc_cells;
use crate::theme::{ACCENT, CURSOR_BG, DIM, TEXT, WARN_GOLD, panel_block};
use crate::widgets::BrailleSpinner;

#[derive(Debug, Clone)]
pub enum ActiveModal {
    CreatePlaylist {
        input: String,
    },
    RenamePlaylist {
        playlist_id: String,
        input: String,
    },
    ImportPlaylistUrl {
        input: String,
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
                    "  No playlists yet — switch to 2 Playlists and press c to create one.",
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
                let row_w = usize::from(inner.width.saturating_sub(2));
                for (i, pl) in playlists
                    .iter()
                    .enumerate()
                    .skip(scroll_start)
                    .take(visible_h)
                {
                    let sel = i == clamped;
                    let prefix = if sel { "▸ " } else { "  " };
                    let bg = if sel { CURSOR_BG } else { Color::Reset };
                    let label = format!("{prefix}{} ({} tracks)", pl.title, pl.track_count);
                    lines.push(Line::from(Span::styled(
                        trunc_cells(&label, row_w),
                        Style::default().fg(if sel { ACCENT } else { TEXT }).bg(bg),
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
                    Style::default().fg(ACCENT).bg(Color::Reset),
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
        ActiveModal::KeymapHelp { scroll } => {
            let rect = centered_rect(84, 22, area);
            frame.render_widget(Clear, rect);
            let block = panel_block(
                "Vim Keymap Reference",
                Some("j/k scroll · Esc/? close"),
                true,
            );
            let inner = block.inner(rect);
            frame.render_widget(block, rect);

            let entries: &[(&str, &str)] = &[
                (
                    "1 - 5 / gt gT",
                    "Switch root tabs (Home · Playlists · Library · Queue · Settings)",
                ),
                (
                    "Tab / Shift+Tab",
                    "Next / previous sub-tab (Mixes/New Releases, Library, Artist)",
                ),
                ("Ctrl+w + h/j/k/l", "Focus split panes"),
                (
                    "h j k l / Arrows",
                    "Move cursor across lists, tables, and cover grids",
                ),
                ("gg / G", "Jump to top / bottom"),
                ("Ctrl+d / Ctrl+u", "Half-page scroll down / up"),
                ("L (Shift+L)", "Toggle expanded Lyrics globally"),
                ("t", "Toggle Indonesian lyrics translation"),
                (
                    "S (Shift+S)",
                    "Sync playlists & library with YouTube Music now",
                ),
                (
                    "/",
                    "Search (Home) or inline filter (Playlists / PlaylistDetail / Library)",
                ),
                ("Space / n / b", "Play-Pause / Next track / Previous track"),
                ("> <  or  ] [", "Seek +5s / -5s"),
                ("+ -", "Volume +5% / -5% (0-130% with lookahead limiter)"),
                ("s / m", "Toggle Shuffle / Cycle Repeat (off -> all -> one)"),
                ("Enter / p", "Play or Open / Play entity directly"),
                ("N / a", "Play Next (priority) / Append to queue"),
                (
                    "P / f / w",
                    "Add to Playlist / Like-Unlike / Save-Unsave Album·Artist·Playlist",
                ),
                (
                    "r / o / v / c",
                    "Start Radio / Cycle Sort / Grid-List / Cover Size (or Create Playlist)",
                ),
                ("dd / Del", "Remove track or delete playlist (with confirm)"),
                (
                    "J / K (Shift)",
                    "Reorder track down / up (Queue & User Playlists)",
                ),
                ("u / D / C", "Undo queue / Deduplicate / Clear upcoming"),
                (
                    "R / I / E / U",
                    "Playlists: Rename / Import URL / Export JSON+M3U8 / Upload to YTM",
                ),
                ("yy / Backspace / q", "Copy YTM URL / Go Back / Clean Quit"),
            ];

            let max_scroll = entries.len().saturating_sub(usize::from(inner.height));
            let clamped_scroll = (*scroll).min(max_scroll);
            let lines: Vec<Line<'_>> = entries
                .iter()
                .skip(clamped_scroll)
                .map(|&(k, desc)| {
                    Line::from(vec![
                        Span::styled(
                            format!("  {k:<18} "),
                            Style::default()
                                .fg(ACCENT)
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
    let shown = if mask && !val.is_empty() {
        "•".repeat(val.chars().count().min(24))
    } else {
        val.to_string()
    };
    let cursor = if focused { "█" } else { "" };
    let bg = if focused { CURSOR_BG } else { Color::Reset };
    Line::from(vec![
        Span::styled(label, Style::default().fg(DIM).bg(Color::Reset)),
        Span::styled(
            format!(" {shown}{cursor} "),
            Style::default()
                .fg(if focused { ACCENT } else { TEXT })
                .bg(bg),
        ),
    ])
}

fn render_text_prompt(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    prompt: &str,
    input: &str,
    hint: &str,
) {
    let rect = centered_rect(64, 8, area);
    frame.render_widget(Clear, rect);
    let block = panel_block(title, Some(hint), true);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let lines = vec![
        Line::from(Span::styled(
            prompt,
            Style::default().fg(DIM).bg(Color::Reset),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("› ", Style::default().fg(ACCENT).bg(Color::Reset)),
            Span::styled(
                format!("{input}█"),
                Style::default()
                    .fg(TEXT)
                    .bg(CURSOR_BG)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
