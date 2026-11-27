use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageBuffer, Rgba};
use lru::LruCache;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, ResizeEncodeRender, StatefulImage};

use musick_core::ImageProtocol;

use crate::theme::{DIM, active_theme};
use crate::widgets::BrailleSpinner;

const MAX_IMAGE_CACHE: NonZeroUsize = NonZeroUsize::MIN.saturating_add(127);
const MAX_PROTO_CACHE: NonZeroUsize = NonZeroUsize::MIN.saturating_add(127);
const WORKER_QUEUE_BOUND: usize = 64;

static KITTY_GRAPHICS_ACTIVE: AtomicBool = AtomicBool::new(false);

type ProtoKey = (String, usize, u16, u16);

struct EncodeReq {
    key: ProtoKey,
    gen_id: u64,
    src: Arc<DynamicImage>,
    picker: Picker,
    pixel_proto: bool,
    font_size: (u16, u16),
    width: u16,
    height: u16,
    resize: Resize,
}

struct EncodeResp {
    key: ProtoKey,
    gen_id: u64,
    proto: StatefulProtocol,
}

/// delete all kitty graphics placements from stdout if kitty graphics were rendered
pub fn write_kitty_delete_all() {
    if KITTY_GRAPHICS_ACTIVE.swap(false, Ordering::SeqCst) {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(b"\x1b_Ga=d,d=A\x1b\\");
        let _ = stdout.flush();
    }
}

fn is_dumb_or_linux_tty() -> bool {
    matches!(
        std::env::var("TERM").ok().as_deref(),
        Some("linux" | "dumb")
    )
}

/// center-crop `src` to 1:1, scale cleanly to `min(canvas_w, canvas_h)`, and center on transparent rgba canvas
pub fn center_square_canvas(src: &DynamicImage, canvas_w: u32, canvas_h: u32) -> DynamicImage {
    let cw = canvas_w.max(1);
    let ch = canvas_h.max(1);
    let side = cw.min(ch).max(1);

    let (sw, sh) = (src.width(), src.height());
    let cropped = if sw != sh && sw > 0 && sh > 0 {
        let s = sw.min(sh);
        src.crop_imm((sw - s) / 2, (sh - s) / 2, s, s)
    } else {
        src.clone()
    };

    let scaled = cropped.resize_exact(side, side, FilterType::Triangle);
    if cw == side && ch == side {
        return scaled;
    }

    let mut canvas: DynamicImage = ImageBuffer::from_pixel(cw, ch, Rgba([0u8, 0, 0, 0])).into();
    let ox = i64::from((cw - side) / 2);
    let oy = i64::from((ch - side) / 2);
    imageops::overlay(&mut canvas, &scaled, ox, oy);
    canvas
}

fn encode_protocol(
    src: &DynamicImage,
    picker: &Picker,
    pixel_proto: bool,
    font_size: (u16, u16),
    width: u16,
    height: u16,
    resize: &Resize,
) -> StatefulProtocol {
    let prepared = if pixel_proto {
        let (fw, fh) = font_size;
        let cw = u32::from(width) * u32::from(fw.max(1));
        let ch = u32::from(height) * u32::from(fh.max(1));
        center_square_canvas(src, cw, ch)
    } else {
        src.clone()
    };
    let mut proto = picker.new_resize_protocol(prepared);
    proto.resize_encode(resize, Size::new(width, height));
    proto
}

fn spawn_encode_worker() -> (mpsc::SyncSender<EncodeReq>, mpsc::Receiver<EncodeResp>) {
    let (req_tx, req_rx) = mpsc::sync_channel::<EncodeReq>(WORKER_QUEUE_BOUND);
    let (resp_tx, resp_rx) = mpsc::channel::<EncodeResp>();
    let _ = std::thread::Builder::new()
        .name("musick-cover-enc".to_string())
        .spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let proto = encode_protocol(
                    &req.src,
                    &req.picker,
                    req.pixel_proto,
                    req.font_size,
                    req.width,
                    req.height,
                    &req.resize,
                );
                if resp_tx
                    .send(EncodeResp {
                        key: req.key,
                        gen_id: req.gen_id,
                        proto,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
    (req_tx, resp_rx)
}

/// manages terminal graphics protocol detection, bounded lru image caches, and off-thread encoding
pub struct CoverRenderer {
    picker: Picker,
    detected_protocol: ProtocolType,
    probed: bool,
    enabled: bool,
    modal_open: bool,
    font_size: (u16, u16),
    images: LruCache<String, Arc<DynamicImage>>,
    protocols: LruCache<ProtoKey, StatefulProtocol>,
    frame_counts: HashMap<String, usize>,
    in_flight: HashSet<ProtoKey>,
    generation: u64,
    enc_tx: Option<mpsc::SyncSender<EncodeReq>>,
    enc_rx: Option<mpsc::Receiver<EncodeResp>>,
}

impl CoverRenderer {
    pub fn detect(pref: ImageProtocol) -> Self {
        let skip_probe = matches!(pref, ImageProtocol::Off) || is_dumb_or_linux_tty();
        let (mut picker, probed) = if skip_probe {
            (Picker::halfblocks(), is_dumb_or_linux_tty())
        } else {
            (
                Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
                true,
            )
        };
        let detected_protocol = picker.protocol_type();
        let fs = picker.font_size();
        let has_native_graphics =
            !is_dumb_or_linux_tty() && !matches!(detected_protocol, ProtocolType::Halfblocks);
        let enabled = match pref {
            ImageProtocol::Off => false,
            ImageProtocol::Auto => has_native_graphics,
            ImageProtocol::Kitty | ImageProtocol::Sixel | ImageProtocol::Halfblock => true,
        };
        match pref {
            ImageProtocol::Auto | ImageProtocol::Off => {}
            ImageProtocol::Kitty => picker.set_protocol_type(ProtocolType::Kitty),
            ImageProtocol::Sixel => picker.set_protocol_type(ProtocolType::Sixel),
            ImageProtocol::Halfblock => picker.set_protocol_type(ProtocolType::Halfblocks),
        }
        let (tx, rx) = spawn_encode_worker();
        Self {
            picker,
            detected_protocol,
            probed,
            enabled,
            modal_open: false,
            font_size: (fs.width, fs.height),
            images: LruCache::new(MAX_IMAGE_CACHE),
            protocols: LruCache::new(MAX_PROTO_CACHE),
            frame_counts: HashMap::new(),
            in_flight: HashSet::new(),
            generation: 1,
            enc_tx: Some(tx),
            enc_rx: Some(rx),
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
            probed: true,
            enabled: true,
            modal_open: false,
            font_size,
            images: LruCache::new(MAX_IMAGE_CACHE),
            protocols: LruCache::new(MAX_PROTO_CACHE),
            frame_counts: HashMap::new(),
            in_flight: HashSet::new(),
            generation: 1,
            enc_tx: None,
            enc_rx: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_modal_open(&mut self, open: bool) {
        self.modal_open = open;
    }

    pub fn has_pending_renders(&self) -> bool {
        !self.in_flight.is_empty()
    }

    pub fn purge_kitty_graphics(&mut self) {
        write_kitty_delete_all();
    }

    pub fn begin_frame(&mut self) {
        self.frame_counts.clear();
        if let Some(ref rx) = self.enc_rx {
            while let Ok(resp) = rx.try_recv() {
                self.in_flight.remove(&resp.key);
                if resp.gen_id == self.generation {
                    self.protocols.put(resp.key, resp.proto);
                }
            }
        }
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

    /// outer column width so the border stroke rectangle `(W - 1) * fw` × `(H - 1) * fh` is a 1:1 visual square
    pub fn square_panel_width(&self, panel_height: u16, max_width: u16) -> u16 {
        let aspect = self.cell_aspect();
        let desired = if panel_height > 1 {
            ((f64::from(panel_height - 1) * aspect).round() as u16).saturating_add(1)
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
        if !matches!(pref, ImageProtocol::Off) && !self.probed && !is_dumb_or_linux_tty() {
            if let Ok(queried) = Picker::from_query_stdio() {
                let fs = queried.font_size();
                self.detected_protocol = queried.protocol_type();
                self.font_size = (fs.width, fs.height);
                self.picker = queried;
            }
            self.probed = true;
        }
        let next_enabled = match pref {
            ImageProtocol::Off => false,
            ImageProtocol::Auto => {
                !is_dumb_or_linux_tty()
                    && !matches!(self.detected_protocol, ProtocolType::Halfblocks)
            }
            ImageProtocol::Kitty | ImageProtocol::Sixel | ImageProtocol::Halfblock => true,
        };
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
            self.generation = self.generation.wrapping_add(1);
            self.protocols.clear();
            self.in_flight.clear();
            self.purge_kitty_graphics();
        }
        changed
    }

    pub fn clear_cache(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.images.clear();
        self.protocols.clear();
        self.in_flight.clear();
        self.frame_counts.clear();
        self.purge_kitty_graphics();
    }

    pub fn insert_image(&mut self, url: String, img: Arc<DynamicImage>) {
        self.images.put(url.clone(), img);
        // drop cached protocols for updated url
        let stale_keys: Vec<ProtoKey> = self
            .protocols
            .iter()
            .filter_map(|(k, _)| if k.0 == url { Some(k.clone()) } else { None })
            .collect();
        for k in stale_keys {
            self.protocols.pop(&k);
        }
        self.in_flight.retain(|k| k.0 != url);
    }

    pub fn has_image(&self, url: &str) -> bool {
        self.images.contains(url)
    }

    fn uses_pixel_protocol(&self) -> bool {
        !matches!(self.picker.protocol_type(), ProtocolType::Halfblocks)
    }

    fn sixel_blocked_by_modal(&self) -> bool {
        self.modal_open && self.picker.protocol_type() == ProtocolType::Sixel
    }

    fn protocol_for_frame(
        &mut self,
        url: &str,
        width: u16,
        height: u16,
        resize: &Resize,
    ) -> Option<&mut StatefulProtocol> {
        if !self.enabled || self.sixel_blocked_by_modal() || !self.images.contains(url) {
            return None;
        }
        let occ = {
            let count = self.frame_counts.entry(url.to_string()).or_insert(0);
            let cur = *count;
            *count += 1;
            cur
        };
        let key: ProtoKey = (url.to_string(), occ, width, height);
        if self.protocols.contains(&key) {
            return self.protocols.get_mut(&key);
        }

        let src = self.images.get(url).cloned()?;
        let pixel_proto = self.uses_pixel_protocol();
        if let Some(ref tx) = self.enc_tx {
            if self.in_flight.insert(key.clone()) {
                let req = EncodeReq {
                    key: key.clone(),
                    gen_id: self.generation,
                    src,
                    picker: self.picker.clone(),
                    pixel_proto,
                    font_size: self.font_size,
                    width,
                    height,
                    resize: resize.clone(),
                };
                if tx.try_send(req).is_err() {
                    self.in_flight.remove(&key);
                }
            }
            return None;
        }

        // synchronous path for deterministic headless unit tests
        let proto = encode_protocol(
            &src,
            &self.picker,
            pixel_proto,
            self.font_size,
            width,
            height,
            resize,
        );
        self.protocols.put(key.clone(), proto);
        self.protocols.get_mut(&key)
    }

    /// 1:1 visual square cell rect centered inside `area` (used for halfblock fallback)
    pub fn square_rect(&self, area: Rect) -> Rect {
        if area.width < 2 || area.height == 0 {
            return Rect::default();
        }
        let aspect = self.cell_aspect();
        let w_for_full_h = (f64::from(area.height) * aspect).round() as u16;
        let (w, h) = if w_for_full_h <= area.width {
            (w_for_full_h.max(2), area.height)
        } else {
            let h_fit = ((f64::from(area.width) / aspect).floor() as u16)
                .max(1)
                .min(area.height);
            let w_fit = ((f64::from(h_fit) * aspect).round() as u16)
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
        let pixel_proto = self.uses_pixel_protocol();
        let target = if pixel_proto {
            area
        } else {
            self.square_rect(area)
        };
        if target.width < 2 || target.height == 0 {
            return;
        }

        let resize = if pixel_proto {
            Resize::Fit(Some(FilterType::Triangle))
        } else {
            Resize::Scale(Some(FilterType::Triangle))
        };
        let is_kitty = self.picker.protocol_type() == ProtocolType::Kitty;
        if self.enabled
            && !self.sixel_blocked_by_modal()
            && let Some(u) = url
            && let Some(proto) = self.protocol_for_frame(u, target.width, target.height, &resize)
        {
            if is_kitty {
                KITTY_GRAPHICS_ACTIVE.store(true, Ordering::Relaxed);
            }
            let widget = StatefulImage::default().resize(resize);
            frame.render_stateful_widget(widget, target, proto);
            return;
        }

        let mid_y = area.y + area.height / 2;
        let encoding = url.is_some_and(|u| self.has_image(u) && !self.sixel_blocked_by_modal());
        let accent = active_theme().accent;
        let label = if self.enabled && (loading || encoding) {
            Line::from(format!("{} loading", spinner.frame()))
                .style(Style::default().fg(accent).bg(Color::Reset))
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
        let resize = Resize::Fit(Some(FilterType::Triangle));
        let is_kitty = self.picker.protocol_type() == ProtocolType::Kitty;
        if self.enabled
            && !self.sixel_blocked_by_modal()
            && let Some(u) = url
            && let Some(proto) = self.protocol_for_frame(u, area.width, area.height, &resize)
        {
            if is_kitty {
                KITTY_GRAPHICS_ACTIVE.store(true, Ordering::Relaxed);
            }
            let widget = StatefulImage::default().resize(resize);
            frame.render_stateful_widget(widget, area, proto);
            return;
        }
        let fg = if highlight {
            active_theme().accent
        } else {
            DIM
        };
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
