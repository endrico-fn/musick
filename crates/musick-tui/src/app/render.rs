use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::theme::{
    BORDER, accent_bold_style, base_style, dim_style, icons_for, set_active_accent,
};
use crate::views::RenderCtx;
use crate::widgets::{
    NowPlayingBar, RootTab, TopBar, compute_topbar_tab_hitboxes, render_modal,
};

use super::{App, NavPage};

impl App {
    fn active_footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        if self.player_bar_focused {
            return &[
                ("Tab", "next ctrl"),
                ("h/l", "scrub/move"),
                ("Enter", "activate"),
                ("A/a", "artist/album"),
                ("Esc/C-w k", "back"),
            ];
        }
        if let Some(top) = self.nav_stack.last() {
            match top {
                NavPage::Artist(v) => v.footer_hints(),
                NavPage::Album(v) => v.footer_hints(),
                NavPage::PlaylistDetail(v) => v.footer_hints(),
            }
        } else {
            match self.active_tab {
                RootTab::Player => self.player.footer_hints(),
                RootTab::Playlists => self.playlists.footer_hints(),
                RootTab::Library => self.library.footer_hints(),
                RootTab::Queue => self.queue.footer_hints(),
                RootTab::Settings => self.settings_view.footer_hints(),
            }
        }
    }

    pub fn render(&mut self, frame: &mut Frame<'_>) {
        set_active_accent(self.settings.display.accent);
        self.covers.begin_frame();
        self.covers.set_modal_open(self.modal.is_some());
        let full = frame.area();
        if full.width < 24 || full.height < 10 {
            return;
        }

        let frame_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(BORDER).bg(Color::Reset))
            .style(Style::default().bg(Color::Reset));
        let inner = frame_block.inner(full);
        frame.render_widget(frame_block, full);

        if inner.width < 20 || inner.height < 8 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(4),
                Constraint::Length(5),
                Constraint::Length(1),
            ])
            .split(inner);

        let icons = icons_for(self.settings.display.icons);
        let auth_mode = self.services.auth.mode();
        let pending_sync = self.pending_sync;
        let toast_str = self.status_toast.as_ref().map(|(s, _)| s.as_str());

        self.topbar_tab_hitboxes = compute_topbar_tab_hitboxes(chunks[0], icons);
        frame.render_widget(
            TopBar {
                active_tab: self.active_tab,
                icons,
                auth_mode,
                syncing: self.syncing,
                pending_sync,
                spinner: self.spinner,
                status_toast: toast_str,
            },
            chunks[0],
        );

        let body_area = inset_h(chunks[1], 1);
        let player_area = inset_h(chunks[2], 1);
        let footer_area = inset_h(chunks[3], 1);

        let ctx = RenderCtx {
            snapshot: &self.snapshot,
            position: self.position,
            queue: &self.queue_snap,
            lyrics: &self.lyrics,
            cava: &self.cava_frame,
            settings: &self.settings,
            icons,
            spinner: self.spinner,
            auth_mode,
            syncing: self.syncing,
            pending_sync,
            cache_stats: &self.cache_stats,
        };

        if let Some(top) = self.nav_stack.last_mut() {
            match top {
                NavPage::Artist(v) => v.render(frame, body_area, &ctx, &mut self.covers),
                NavPage::Album(v) => v.render(frame, body_area, &ctx, &mut self.covers),
                NavPage::PlaylistDetail(v) => v.render(frame, body_area, &ctx, &mut self.covers),
            }
        } else {
            match self.active_tab {
                RootTab::Player => self.player.render(frame, body_area, &ctx, &mut self.covers),
                RootTab::Playlists => self.playlists.render(frame, body_area, &ctx),
                RootTab::Library => self
                    .library
                    .render(frame, body_area, &ctx, &mut self.covers),
                RootTab::Queue => self.queue.render(frame, body_area, &ctx),
                RootTab::Settings => self.settings_view.render(frame, body_area, &ctx),
            }
        }

        let selected_ctrl = self.player_bar_focused.then_some(self.player_bar_control);
        self.player_bar_hitboxes = NowPlayingBar {
            snapshot: &self.snapshot,
            position: self.position,
            icons,
            spinner: self.spinner,
            focused: self.player_bar_focused,
        }
        .render_with_hitboxes(player_area, frame.buffer_mut(), selected_ctrl);

        let mut f_spans = Vec::new();
        if let Some(chord) = self.keymap.pending_label() {
            f_spans.push(Span::styled(format!("[{chord}] "), accent_bold_style()));
        }
        for (i, &(k, label)) in self.active_footer_hints().iter().enumerate() {
            if i > 0 {
                f_spans.push(Span::styled("  ", dim_style()));
            }
            f_spans.push(Span::styled(k, base_style().add_modifier(Modifier::BOLD)));
            f_spans.push(Span::styled(format!(" {label}"), dim_style()));
        }
        frame.render_widget(Paragraph::new(Line::from(f_spans)), footer_area);

        if let Some(ref modal) = self.modal {
            render_modal(frame, full, modal, self.spinner);
        }
    }
}

fn inset_h(r: Rect, pad: u16) -> Rect {
    let double = pad.saturating_mul(2);
    if r.width <= double {
        return r;
    }
    Rect {
        x: r.x + pad,
        y: r.y,
        width: r.width - double,
        height: r.height,
    }
}
