use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use image::DynamicImage;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};

use musick_core::ImageProtocol;

use crate::theme::{ACCENT, DIM};
use crate::widgets::BrailleSpinner;

const MAX_PROTO_CACHE: usize = 64;

/// Manages terminal graphics protocol detection and bounded stateful image cache.
pub struct CoverRenderer {
    picker: Picker,
    detected_protocol: ProtocolType,
    font_size: (u16, u16),
    protocols: HashMap<String, StatefulProtocol>,
    order: VecDeque<String>,
}

impl CoverRenderer {
    pub fn detect(pref: ImageProtocol) -> Self {
        let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        let detected_protocol = picker.protocol_type();
        let fs = picker.font_size();
        match pref {
            ImageProtocol::Auto => {}
            ImageProtocol::Kitty => picker.set_protocol_type(ProtocolType::Kitty),
            ImageProtocol::Sixel => picker.set_protocol_type(ProtocolType::Sixel),
            ImageProtocol::Halfblock => picker.set_protocol_type(ProtocolType::Halfblocks),
        }
        Self {
            picker,
            detected_protocol,
            font_size: (fs.width, fs.height),
            protocols: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn halfblocks_for_test() -> Self {
        Self::with_font_size((8, 16))
    }

    pub fn with_font_size(font_size: (u16, u16)) -> Self {
        let picker = Picker::halfblocks();
        Self {
            picker,
            detected_protocol: ProtocolType::Halfblocks,
            font_size,
            protocols: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Return terminal cell pixel aspect ratio (`font_height / font_width`), clamped to `1.5..=3.0`.
    pub fn cell_aspect(&self) -> f64 {
        let (fw, fh) = self.font_size;
        if fw > 0 && fh > 0 {
            (f64::from(fh) / f64::from(fw)).clamp(1.5, 3.0)
        } else {
            2.0
        }
    }

    /// Compute the outer column width so a bordered panel of `panel_height` rows forms a 1:1 visual square.
    pub fn square_panel_width(&self, panel_height: u16, max_width: u16) -> u16 {
        let desired = (f64::from(panel_height) * self.cell_aspect()).round() as u16;
        desired.min(max_width).max(6.min(max_width))
    }

    pub fn set_protocol(&mut self, pref: ImageProtocol) -> bool {
        let proto = match pref {
            ImageProtocol::Auto => self.detected_protocol,
            ImageProtocol::Kitty => ProtocolType::Kitty,
            ImageProtocol::Sixel => ProtocolType::Sixel,
            ImageProtocol::Halfblock => ProtocolType::Halfblocks,
        };
        if proto != self.picker.protocol_type() {
            self.picker.set_protocol_type(proto);
            self.protocols.clear();
            self.order.clear();
            return true;
        }
        false
    }

    pub fn clear_cache(&mut self) {
        self.protocols.clear();
        self.order.clear();
    }

    pub fn insert_image(&mut self, url: String, img: Arc<DynamicImage>) {
        let proto = self.picker.new_resize_protocol((*img).clone());
        self.protocols.insert(url.clone(), proto);
        self.order.retain(|k| k != &url);
        self.order.push_back(url);
        while self.protocols.len() > MAX_PROTO_CACHE {
            if let Some(oldest) = self.order.pop_front() {
                self.protocols.remove(&oldest);
            } else {
                break;
            }
        }
    }

    pub fn has_image(&self, url: &str) -> bool {
        self.protocols.contains_key(url)
    }

    /// Compute a 1:1 visual square centered inside `area` using the actual terminal cell aspect ratio.
    pub fn square_rect(&self, area: Rect) -> Rect {
        if area.width < 2 || area.height == 0 {
            return Rect::default();
        }
        let aspect = self.cell_aspect();
        let w_for_full_h = (f64::from(area.height) * aspect).ceil() as u16;
        let (w, h) = if w_for_full_h <= area.width {
            (w_for_full_h.max(2), area.height)
        } else {
            let h_fit = ((f64::from(area.width) / aspect).floor() as u16)
                .max(1)
                .min(area.height);
            let w_fit = ((f64::from(h_fit) * aspect).ceil() as u16)
                .min(area.width)
                .max(2);
            (w_fit, h_fit)
        };
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    pub fn render_cover(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        url: Option<&str>,
        loading: bool,
        placeholder_glyph: &str,
        spinner: BrailleSpinner,
    ) {
        if area.width < 2 || area.height == 0 {
            return;
        }
        let target = self.square_rect(area);
        if target.width < 2 || target.height == 0 {
            return;
        }

        if let Some(u) = url
            && let Some(proto) = self.protocols.get_mut(u)
        {
            let widget = StatefulImage::default().resize(Resize::Fit(None));
            frame.render_stateful_widget(widget, target, proto);
            return;
        }

        let mid_y = area.y + area.height / 2;
        let label = if loading {
            Line::from(format!("{} loading", spinner.frame()))
                .style(Style::default().fg(ACCENT).bg(Color::Reset))
        } else {
            Line::from(placeholder_glyph.to_string())
                .style(Style::default().fg(DIM).bg(Color::Reset))
        };
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center),
            Rect {
                x: area.x,
                y: mid_y,
                width: area.width,
                height: 1,
            },
        );
    }
}
