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

const MAX_IMAGE_CACHE: usize = 128;
const MAX_PROTO_CACHE: usize = 128;

/// manages terminal graphics protocol detection and bounded stateful image cache
pub struct CoverRenderer {
    picker: Picker,
    detected_protocol: ProtocolType,
    enabled: bool,
    font_size: (u16, u16),
    images: HashMap<String, Arc<DynamicImage>>,
    image_order: VecDeque<String>,
    protocols: HashMap<(String, usize), StatefulProtocol>,
    proto_order: VecDeque<(String, usize)>,
    frame_counts: HashMap<String, usize>,
}

impl CoverRenderer {
    pub fn detect(pref: ImageProtocol) -> Self {
        let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        let detected_protocol = picker.protocol_type();
        let fs = picker.font_size();
        let enabled = !matches!(pref, ImageProtocol::Off);
        match pref {
            ImageProtocol::Auto | ImageProtocol::Off => {}
            ImageProtocol::Kitty => picker.set_protocol_type(ProtocolType::Kitty),
            ImageProtocol::Sixel => picker.set_protocol_type(ProtocolType::Sixel),
            ImageProtocol::Halfblock => picker.set_protocol_type(ProtocolType::Halfblocks),
        }
        Self {
            picker,
            detected_protocol,
            enabled,
            font_size: (fs.width, fs.height),
            images: HashMap::new(),
            image_order: VecDeque::new(),
            protocols: HashMap::new(),
            proto_order: VecDeque::new(),
            frame_counts: HashMap::new(),
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
            enabled: true,
            font_size,
            images: HashMap::new(),
            image_order: VecDeque::new(),
            protocols: HashMap::new(),
            proto_order: VecDeque::new(),
            frame_counts: HashMap::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn begin_frame(&mut self) {
        self.frame_counts.clear();
    }

    /// terminal cell pixel aspect ratio (`font_height / font_width`), clamped to `1.5..=3.0`
    pub fn cell_aspect(&self) -> f64 {
        let (fw, fh) = self.font_size;
        if fw > 0 && fh > 0 {
            (f64::from(fh) / f64::from(fw)).clamp(1.5, 3.0)
        } else {
            2.0
        }
    }

    /// outer column width so a bordered panel with 1-cell horizontal padding forms a 1:1 visual square
    pub fn square_panel_width(&self, panel_height: u16, max_width: u16) -> u16 {
        let aspect = self.cell_aspect();
        let desired = if panel_height > 2 {
            ((f64::from(panel_height - 2) * aspect).ceil() as u16).saturating_add(4)
        } else {
            (f64::from(panel_height) * aspect).round() as u16
        };
        desired.min(max_width).max(6.min(max_width))
    }

    /// column width for a 1:1 inline mini cover of `rows` rows
    pub fn mini_cover_width(&self, rows: u16) -> u16 {
        ((f64::from(rows) * self.cell_aspect()).round() as u16).clamp(2, 8)
    }

    pub fn set_protocol(&mut self, pref: ImageProtocol) -> bool {
        let next_enabled = !matches!(pref, ImageProtocol::Off);
        let proto = match pref {
            ImageProtocol::Auto | ImageProtocol::Off => self.detected_protocol,
            ImageProtocol::Kitty => ProtocolType::Kitty,
            ImageProtocol::Sixel => ProtocolType::Sixel,
            ImageProtocol::Halfblock => ProtocolType::Halfblocks,
        };
        let changed = next_enabled != self.enabled || proto != self.picker.protocol_type();
        self.enabled = next_enabled;
        if proto != self.picker.protocol_type() {
            self.picker.set_protocol_type(proto);
        }
        if changed {
            self.protocols.clear();
            self.proto_order.clear();
        }
        changed
    }

    pub fn clear_cache(&mut self) {
        self.images.clear();
        self.image_order.clear();
        self.protocols.clear();
        self.proto_order.clear();
        self.frame_counts.clear();
    }

    pub fn insert_image(&mut self, url: String, img: Arc<DynamicImage>) {
        self.images.insert(url.clone(), img);
        self.image_order.retain(|k| k != &url);
        self.image_order.push_back(url.clone());
        while self.images.len() > MAX_IMAGE_CACHE {
            if let Some(oldest) = self.image_order.pop_front() {
                self.images.remove(&oldest);
                self.protocols.retain(|(u, _), _| u != &oldest);
                self.proto_order.retain(|(u, _)| u != &oldest);
            } else {
                break;
            }
        }
        // refresh any cached stateful protocols for updated image bytes
        self.protocols.retain(|(u, _), _| u != &url);
        self.proto_order.retain(|(u, _)| u != &url);
    }

    pub fn has_image(&self, url: &str) -> bool {
        self.images.contains_key(url)
    }

    fn protocol_for_frame(&mut self, url: &str) -> Option<&mut StatefulProtocol> {
        if !self.enabled || !self.images.contains_key(url) {
            return None;
        }
        let occ = {
            let count = self.frame_counts.entry(url.to_string()).or_insert(0);
            let cur = *count;
            *count += 1;
            cur
        };
        let key = (url.to_string(), occ);
        if !self.protocols.contains_key(&key) {
            let img = self.images.get(url)?;
            let proto = self.picker.new_resize_protocol((**img).clone());
            self.protocols.insert(key.clone(), proto);
            self.proto_order.push_back(key.clone());
            while self.protocols.len() > MAX_PROTO_CACHE {
                if let Some(oldest) = self.proto_order.pop_front() {
                    if oldest != key {
                        self.protocols.remove(&oldest);
                    }
                } else {
                    break;
                }
            }
        }
        self.protocols.get_mut(&key)
    }

    /// 1:1 visual square centered inside `area` using the actual terminal cell aspect ratio
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

        if self.enabled
            && let Some(u) = url
            && let Some(proto) = self.protocol_for_frame(u)
        {
            let widget = StatefulImage::default().resize(Resize::Fit(None));
            frame.render_stateful_widget(widget, target, proto);
            return;
        }

        let mid_y = area.y + area.height / 2;
        let label = if self.enabled && loading {
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

    /// render a compact 1:1 mini cover or fallback glyph inside `area`
    pub fn render_mini_cover(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        url: Option<&str>,
        fallback_glyph: &str,
        highlight: bool,
    ) {
        if area.width < 2 || area.height == 0 {
            return;
        }
        if self.enabled
            && let Some(u) = url
            && let Some(proto) = self.protocol_for_frame(u)
        {
            let widget = StatefulImage::default().resize(Resize::Fit(None));
            frame.render_stateful_widget(widget, area, proto);
            return;
        }
        let fg = if highlight { ACCENT } else { DIM };
        frame.render_widget(
            Paragraph::new(
                Line::from(fallback_glyph.to_string())
                    .style(Style::default().fg(fg).bg(Color::Reset)),
            )
            .alignment(Alignment::Center),
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
        );
    }
}
