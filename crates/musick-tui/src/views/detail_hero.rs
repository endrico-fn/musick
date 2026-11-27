use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::theme::cover_panel_block;
use crate::widgets::{BrailleSpinner, CoverRenderer};

pub struct DetailHeroLayout {
    pub body_area: Rect,
    pub cover_area: Option<Rect>,
    pub info_area: Rect,
}

/// split a detail view into a responsive hero row and body, collapsing the cover box when images are off
pub fn split_detail_hero(area: Rect, covers: &CoverRenderer) -> DetailHeroLayout {
    let hero_h = if area.height >= 24 {
        9
    } else if area.height >= 16 {
        7
    } else {
        5
    };
    let v_parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(hero_h), Constraint::Min(4)])
        .split(area);

    if !covers.is_enabled() {
        return DetailHeroLayout {
            body_area: v_parts[1],
            cover_area: None,
            info_area: v_parts[0],
        };
    }

    let max_cov_w = area.width.saturating_sub(20).min(area.width / 3).max(10);
    let cov_w = covers.square_panel_width(hero_h, max_cov_w);
    let hero_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(cov_w),
            Constraint::Length(1),
            Constraint::Min(16),
        ])
        .split(v_parts[0]);

    DetailHeroLayout {
        body_area: v_parts[1],
        cover_area: Some(hero_cols[0]),
        info_area: hero_cols[2],
    }
}

pub struct DetailCover<'a> {
    pub area: Option<Rect>,
    pub title: &'a str,
    pub url: Option<&'a str>,
    pub entity_loading: bool,
    pub placeholder_glyph: &'a str,
}

pub fn render_detail_cover(
    frame: &mut Frame<'_>,
    spec: DetailCover<'_>,
    spinner: BrailleSpinner,
    covers: &mut CoverRenderer,
) {
    let Some(rect) = spec.area else {
        return;
    };
    let cov_block = cover_panel_block(spec.title, None, false);
    let cov_inner = cov_block.inner(rect);
    frame.render_widget(cov_block, rect);
    let cover_loading = spec.entity_loading || spec.url.is_some_and(|u| !covers.has_image(u));
    covers.render_cover(
        frame,
        cov_inner,
        spec.url,
        cover_loading,
        spec.placeholder_glyph,
        spinner,
    );
}
