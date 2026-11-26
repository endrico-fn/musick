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
    protocols: HashMap<String, StatefulProtocol>,
    order: VecDeque<String>,
}

impl CoverRenderer {
    pub fn detect(pref: ImageProtocol) -> Self {
        let mut picker = match pref {
            ImageProtocol::Halfblock => Picker::halfblocks(),
            ImageProtocol::Auto => Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
            ImageProtocol::Kitty => {
                let mut p = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
                p.set_protocol_type(ProtocolType::Kitty);
                p
            }
            ImageProtocol::Sixel => {
                let mut p = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
                p.set_protocol_type(ProtocolType::Sixel);
                p
            }
        };
        if pref == ImageProtocol::Halfblock {
            picker.set_protocol_type(ProtocolType::Halfblocks);
        }
        Self {
            picker,
            protocols: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn halfblocks_for_test() -> Self {
        Self {
            picker: Picker::halfblocks(),
            protocols: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn set_protocol(&mut self, pref: ImageProtocol) {
        let proto = match pref {
            ImageProtocol::Auto => self.picker.protocol_type(),
            ImageProtocol::Kitty => ProtocolType::Kitty,
            ImageProtocol::Sixel => ProtocolType::Sixel,
            ImageProtocol::Halfblock => ProtocolType::Halfblocks,
        };
        if proto != self.picker.protocol_type() {
            self.picker.set_protocol_type(proto);
            self.protocols.clear();
            self.order.clear();
        }
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

    /// Compute a 1:1 visual square centered inside `area` (terminal cell height ≈ 2× width).
    pub fn square_rect(area: Rect) -> Rect {
        if area.width < 2 || area.height == 0 {
            return Rect::default();
        }
        let max_h = area.height;
        let max_w_from_h = max_h.saturating_mul(2);
        let (w, h) = if max_w_from_h <= area.width {
            (max_w_from_h, max_h)
        } else {
            let h_fit = (area.width / 2).max(1);
            (h_fit.saturating_mul(2).min(area.width), h_fit)
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
        let target = Self::square_rect(area);
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
