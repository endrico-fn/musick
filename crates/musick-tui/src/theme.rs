use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders};

use musick_core::{IconMode, SearchItem};

pub const ACCENT: Color = Color::Rgb(0x26, 0xd9, 0xb4);
pub const ACCENT_BRIGHT: Color = Color::Rgb(0x4e, 0xf0, 0xce);
pub const BORDER: Color = Color::Rgb(0xa6, 0xa6, 0xa6);
pub const BORDER_DIM: Color = Color::Rgb(0x7c, 0x7c, 0x7c);
pub const TEXT: Color = Color::Rgb(0xf2, 0xf2, 0xf2);
pub const DIM: Color = Color::Rgb(0x7c, 0x7c, 0x7c);
pub const TR_ACTIVE: Color = Color::Rgb(0x9e, 0xf5, 0xe2);
pub const TR_UPCOMING: Color = Color::Rgb(0x7d, 0xa8, 0xa0);
pub const TR_PAST: Color = Color::Rgb(0x55, 0x66, 0x63);
pub const CURSOR_BG: Color = Color::Rgb(0x1f, 0x4f, 0x46);
pub const CURSOR_BLUR_BG: Color = Color::Rgb(0x16, 0x35, 0x2f);
pub const BTN_BG: Color = Color::Rgb(0x1d, 0x38, 0x33);
pub const BTN_PRIMARY_FG: Color = Color::Rgb(0x08, 0x13, 0x0f);
pub const WARN_GOLD: Color = Color::Rgb(0xe0, 0xaf, 0x68);
pub const ERR: Color = Color::Rgb(0xe0, 0x6c, 0x75);

#[derive(Debug, Clone, Copy)]
pub struct IconSet {
    pub brand: &'static str,
    pub cursor: &'static str,
    pub playing: &'static str,
    pub pause: &'static str,
    pub prev: &'static str,
    pub next: &'static str,
    pub song: &'static str,
    pub video: &'static str,
    pub album: &'static str,
    pub artist: &'static str,
    pub playlist: &'static str,
    pub prompt: &'static str,
    pub dot: &'static str,
    pub lyrics: &'static str,
    pub shuffle: &'static str,
    pub repeat_off: &'static str,
    pub repeat_all: &'static str,
    pub repeat_one: &'static str,
    pub volume: &'static str,
    pub sort_desc: &'static str,
    pub check_on: &'static str,
    pub check_off: &'static str,
    pub liked: &'static str,
    pub cloud: &'static str,
    pub local: &'static str,
}

impl IconSet {
    pub fn icon_for_item(&self, item: &SearchItem) -> &'static str {
        match item {
            SearchItem::Track(t) => {
                if t.is_video() {
                    self.video
                } else {
                    self.song
                }
            }
            SearchItem::Album(_) => self.album,
            SearchItem::Artist(_) => self.artist,
            SearchItem::Playlist(_) => self.playlist,
        }
    }
}

pub const UNICODE_ICONS: IconSet = IconSet {
    brand: "◆",
    cursor: "▸",
    playing: "▶",
    pause: "▌▌",
    prev: "◀◀",
    next: "▶▶",
    song: "♪",
    video: "▷",
    album: "◎",
    artist: "◉",
    playlist: "≡",
    prompt: "›",
    dot: "●",
    lyrics: "▪",
    shuffle: "SHF",
    repeat_off: "RPT",
    repeat_all: "RPT*",
    repeat_one: "RPT1",
    volume: "VOL",
    sort_desc: "▾",
    check_on: "[x]",
    check_off: "[ ]",
    liked: "♥",
    cloud: "☁",
    local: "▪",
};

pub const NERD_ICONS: IconSet = IconSet {
    brand: "\u{f001}",
    cursor: "▸",
    playing: "\u{f04b}",
    pause: "\u{f04c}",
    prev: "◀◀",
    next: "▶▶",
    song: "\u{f001}",
    video: "\u{f03d}",
    album: "\u{f192}",
    artist: "\u{f007}",
    playlist: "\u{f0ca}",
    prompt: "\u{f002}",
    dot: "●",
    lyrics: "\u{f001}",
    shuffle: "SHF",
    repeat_off: "\u{f01e}",
    repeat_all: "\u{f01e}*",
    repeat_one: "\u{f01e}1",
    volume: "\u{f028}",
    sort_desc: "▾",
    check_on: "[✓]",
    check_off: "[ ]",
    liked: "\u{f004}",
    cloud: "\u{f0c2}",
    local: "\u{f0c7}",
};

pub fn icons_for(mode: IconMode) -> IconSet {
    match mode {
        IconMode::Unicode => UNICODE_ICONS,
        IconMode::Nerd => NERD_ICONS,
    }
}

pub fn base_style() -> Style {
    Style::default().fg(TEXT).bg(Color::Reset)
}

pub fn dim_style() -> Style {
    Style::default().fg(DIM).bg(Color::Reset)
}

pub fn accent_style() -> Style {
    Style::default().fg(ACCENT).bg(Color::Reset)
}

pub fn accent_bold_style() -> Style {
    Style::default()
        .fg(ACCENT)
        .bg(Color::Reset)
        .add_modifier(Modifier::BOLD)
}

pub fn cursor_style(focused: bool) -> Style {
    Style::default()
        .fg(TEXT)
        .bg(if focused { CURSOR_BG } else { CURSOR_BLUR_BG })
}

pub fn panel_block<'a>(title: &'a str, subtitle: Option<&'a str>, focused: bool) -> Block<'a> {
    let border_color = if focused { ACCENT } else { BORDER };
    let title_color = if focused { ACCENT } else { TEXT };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(border_color).bg(Color::Reset))
        .style(Style::default().bg(Color::Reset))
        .title(
            Line::from(format!(" {title} ")).style(
                Style::default()
                    .fg(title_color)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ),
        );

    if let Some(sub) = subtitle.filter(|s| !s.is_empty()) {
        block = block.title_top(
            Line::from(format!(" {sub} "))
                .right_aligned()
                .style(Style::default().fg(ACCENT).bg(Color::Reset)),
        );
    }
    block
}
